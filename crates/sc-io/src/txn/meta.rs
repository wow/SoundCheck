//! File metadata a rename does not carry, the identity of a file, and the checks that decide
//! whether a file may be replaced in place.
//!
//! A new file renamed over an old one has the new file's metadata: no extended attributes (on
//! macOS: Finder tags and colour label, Finder comment, "Where from", quarantine, the resource
//! fork), a new creation date, the default mode and group. [`snapshot`] reads them from the
//! original before the rename and [`restore`] writes them onto the new file after it: every
//! extended attribute byte for byte (each read at its full size: `getxattr(2)` with no buffer
//! gives the size, which matters for the resource fork, whose reads are silently cut to the
//! buffer), read back to check it, then the group, the creation date (macOS
//! `setattrlist(ATTR_CMN_CRTIME)` through std), the modification time when asked, and the mode
//! last. Access control lists are not carried; files that have one are refused in place
//! instead ([`has_acl`]).
//!
//! On macOS the resource fork of a regular file is always read through `<file>/..namedfork/rsrc`,
//! never trusted from the attribute calls alone: while a descriptor that wrote the fork there is
//! still open (in another app, or in a child process that holds a duplicate for the instant it
//! is being spawned), APFS answers `listxattr(2)`/`getxattr(2)` from the last committed state
//! (a new fork is not listed; a fork extended from 3,000 to 83,000 bytes still reads 3,000),
//! while the named fork reads the current bytes and `clonefile(2)`/`copyfile(3)` copy them, so
//! the backup would have them and the new file not. Writing a fork does not shorten a longer
//! one already there (`setxattr(2)` at position 0 overwrites the start only), so [`restore`]
//! removes a fork that differs before writing it.

use std::ffi::{OsStr, OsString};
use std::fs::{FileTimes, Metadata, OpenOptions};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

use sc_core::Result;

use super::fsx::io_err;

/// What identifies one version of a file: replacing it, writing to it or changing its
/// metadata changes at least one field (the change time moves on every write and every
/// metadata change, even when the modification time is set back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileId {
    /// Device.
    pub dev: u64,
    /// Inode.
    pub ino: u64,
    /// Length, bytes.
    pub len: u64,
    /// Modification time, seconds and nanoseconds.
    pub mtime: (i64, i64),
    /// Status change time, seconds and nanoseconds.
    pub ctime: (i64, i64),
}

impl FileId {
    /// The identity in `meta`.
    pub fn of(meta: &Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
        }
    }

    /// The identity of the file at `path` now.
    ///
    /// # Errors
    /// [`sc_core::Error::Io`] naming `path`.
    pub fn read(path: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| io_err(path, e))?;
        Ok(Self::of(&meta))
    }
}

/// The metadata of a file that SoundCheck restores after replacing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileMeta {
    /// The file's identity when it was read (to notice a change, not restored).
    pub id: FileId,
    /// Length, bytes.
    pub len: u64,
    /// Permission bits (`st_mode & 0o7777`).
    pub mode: u32,
    /// Owner.
    pub uid: u32,
    /// Group.
    pub gid: u32,
    /// Modification time.
    pub modified: SystemTime,
    /// Creation (birth) time, where the system reports one.
    pub created: Option<SystemTime>,
    /// Every extended attribute, by name in byte order.
    pub xattrs: Vec<(OsString, Vec<u8>)>,
}

/// The inode and birth time (seconds since 1970, where the system reports it) of the folder
/// `dir`: with the volume's identity, what tells the same folder from another one at the same
/// path (a mount point left empty after its volume went away).
pub(crate) fn folder_id(dir: &Path) -> Option<(u64, Option<i64>)> {
    let meta = std::fs::metadata(dir).ok()?;
    let birth = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());
    Some((meta.ino(), birth))
}

/// Whether a file owned by `file_uid` must not be replaced by a process running as `euid`:
/// the replacement would belong to `euid` (only the superuser may give a file away).
pub(crate) fn owned_by_other(file_uid: u32, euid: u32) -> bool {
    euid != 0 && file_uid != euid
}

/// This process's effective user id.
pub(crate) fn effective_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// The error for a missing extended attribute (`ENOATTR` on Apple systems, `ENODATA` on
/// Linux).
#[cfg(target_os = "macos")]
const NO_ATTR: rustix::io::Errno = rustix::io::Errno::NOATTR;
#[cfg(not(target_os = "macos"))]
const NO_ATTR: rustix::io::Errno = rustix::io::Errno::NODATA;

/// The extended attribute `name` of `path` (not following a link) at its full size, or `None`
/// when it does not exist.
///
/// # Errors
/// The system's error.
pub(crate) fn read_xattr(path: &Path, name: &OsStr) -> std::io::Result<Option<Vec<u8>>> {
    use rustix::io::Errno;
    // The size can grow between the two calls; try again a few times.
    for _ in 0..4 {
        let size = match rustix::fs::lgetxattr(path, name, &mut [0_u8; 0][..]) {
            Ok(n) => n,
            Err(e) if e == NO_ATTR => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut value = vec![0_u8; size];
        match rustix::fs::lgetxattr(path, name, &mut value[..]) {
            Ok(n) if n == size => return Ok(Some(value)),
            // Shrank meanwhile: ask again.
            Ok(_) => {}
            Err(e) if e == NO_ATTR => return Ok(None),
            Err(e) if e == Errno::RANGE => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(std::io::Error::other(format!(
        "extended attribute {} keeps changing size",
        name.to_string_lossy()
    )))
}

/// Reads the metadata of the file at `path` (not following a symbolic link).
///
/// # Errors
/// [`sc_core::Error::Io`] naming `path`.
pub(crate) fn snapshot(path: &Path) -> Result<FileMeta> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| io_err(path, e))?;
    let modified = meta.modified().map_err(|e| io_err(path, e))?;
    let mut names: Vec<OsString> = match xattr::list(path) {
        Ok(list) => list.collect(),
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported => Vec::new(),
        Err(e) => return Err(io_err(path, e)),
    };
    names.sort();
    let mut xattrs = Vec::with_capacity(names.len());
    for name in names {
        match read_xattr(path, &name) {
            Ok(Some(value)) => xattrs.push((name, value)),
            // Removed between the list and the read.
            Ok(None) => {}
            Err(e) => return Err(io_err(path, e)),
        }
    }
    #[cfg(target_os = "macos")]
    if meta.is_file() {
        take_resource_fork(path, &mut xattrs).map_err(|e| io_err(path, e))?;
    }
    Ok(FileMeta {
        id: FileId::of(&meta),
        len: meta.len(),
        mode: meta.mode() & 0o7777,
        uid: meta.uid(),
        gid: meta.gid(),
        modified,
        created: meta.created().ok(),
        xattrs,
    })
}

/// The extended attribute that holds a file's resource fork (`XATTR_RESOURCEFORK_NAME`,
/// sys/xattr.h).
#[cfg(target_os = "macos")]
const RESOURCE_FORK: &str = "com.apple.ResourceFork";

/// The current resource fork of the regular file at `path`, read through its named fork (see
/// the module documentation), empty when it has none (HFS+ shows every file an empty one);
/// `None` when the named fork does not answer: not found, or a volume without named forks.
///
/// # Errors
/// The system's error, saying the named fork was read, for any other failure.
#[cfg(target_os = "macos")]
fn named_fork(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use rustix::io::Errno;
    match std::fs::read(path.join("..namedfork/rsrc")) {
        Ok(fork) => Ok(Some(fork)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e)
            if [
                NO_ATTR,
                Errno::NOTDIR,
                Errno::NOTSUP,
                Errno::OPNOTSUPP,
                Errno::INVAL,
            ]
            .iter()
            .any(|n| e.raw_os_error() == Some(n.raw_os_error())) =>
        {
            Ok(None)
        }
        Err(e) => Err(std::io::Error::new(
            e.kind(),
            format!("its resource fork (read through ..namedfork/rsrc) is not readable: {e}"),
        )),
    }
}

/// Puts the current resource fork of the regular file at `path` into `xattrs` (sorted by
/// name), replacing what the attribute calls returned for it, or removes it there when the
/// named fork is empty. When the named fork does not answer, `xattrs` stays as listed.
///
/// # Errors
/// The system's error for any other failure to read the named fork.
#[cfg(target_os = "macos")]
fn take_resource_fork(path: &Path, xattrs: &mut Vec<(OsString, Vec<u8>)>) -> std::io::Result<()> {
    let name = OsStr::new(RESOURCE_FORK);
    let Some(fork) = named_fork(path)? else {
        return Ok(());
    };
    match xattrs.binary_search_by(|(n, _)| n.as_os_str().cmp(name)) {
        Ok(i) if fork.is_empty() => {
            xattrs.remove(i);
        }
        Ok(i) => {
            if xattrs[i].1 != fork {
                tracing::debug!(path = %path.display(), listed = xattrs[i].1.len(), bytes = fork.len(), "resource fork read through its named fork (the attribute is stale)");
                xattrs[i].1 = fork;
            }
        }
        Err(_) if fork.is_empty() => {}
        Err(at) => {
            tracing::debug!(path = %path.display(), bytes = fork.len(), "resource fork read through its named fork (not listed yet)");
            xattrs.insert(at, (name.to_os_string(), fork));
        }
    }
    Ok(())
}

/// The value of the extended attribute `name` of `path` as readers see it: on macOS the
/// resource fork of a regular file through its named fork (see the module documentation;
/// `None` when it is empty), anything else, or a fork the named fork does not answer for,
/// through [`read_xattr`].
fn read_current(path: &Path, name: &OsStr) -> std::io::Result<Option<Vec<u8>>> {
    #[cfg(target_os = "macos")]
    if name == RESOURCE_FORK
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
        && let Some(fork) = named_fork(path)?
    {
        return Ok(Some(fork).filter(|f| !f.is_empty()));
    }
    read_xattr(path, name)
}

/// Writes `meta` onto the file at `path`: every extended attribute (each read back and
/// compared), the group, then the creation date and (with `keep_mtime`) the modification time,
/// then the mode. Returns notes for what the volume or the system refused (a protected
/// `com.apple.*` name, an attribute that reads back differently, a group this user is not in);
/// the rest is restored.
///
/// # Errors
/// [`sc_core::Error::Io`] naming `path` when the times or the mode cannot be set.
pub(crate) fn restore(path: &Path, meta: &FileMeta, keep_mtime: bool) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    for (name, value) in &meta.xattrs {
        let shown = name.to_string_lossy();
        let current = read_current(path, name);
        if matches!(&current, Ok(Some(v)) if v == value) {
            continue;
        }
        // A fork is overwritten from its start, never shortened: remove a different one first.
        if is_resource_fork(name)
            && matches!(current, Ok(Some(_)))
            && let Err(e) = xattr::remove(path, name)
        {
            notes.push(format!("extended attribute not restored: {shown} ({e})"));
            continue;
        }
        if let Err(e) = xattr::set(path, name, value) {
            tracing::warn!(path = %path.display(), name = %shown, error = %e, "extended attribute not restored");
            notes.push(format!("extended attribute not restored: {shown} ({e})"));
            continue;
        }
        match read_current(path, name) {
            Ok(Some(v)) if v == *value => {}
            Ok(v) => notes.push(format!(
                "extended attribute {shown} reads back as {} bytes, {} were written",
                v.map_or(0, |v| v.len()),
                value.len()
            )),
            Err(e) => notes.push(format!("extended attribute {shown} not readable: {e}")),
        }
    }
    let now = std::fs::symlink_metadata(path).map_err(|e| io_err(path, e))?;
    if now.gid() != meta.gid
        && let Err(e) = std::os::unix::fs::chown(path, None, Some(meta.gid))
    {
        notes.push(format!("group {} not restored: {e}", meta.gid));
    }
    let mut times = FileTimes::new();
    if keep_mtime {
        times = times.set_modified(meta.modified);
    }
    times = with_created(times, meta.created);
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| io_err(path, e))?;
    file.set_times(times).map_err(|e| io_err(path, e))?;
    drop(file);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(meta.mode))
        .map_err(|e| io_err(path, e))?;
    Ok(notes)
}

/// Whether `name` is the resource fork's attribute (macOS only).
fn is_resource_fork(name: &OsStr) -> bool {
    #[cfg(target_os = "macos")]
    {
        name == RESOURCE_FORK
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = name;
        false
    }
}

#[cfg(target_os = "macos")]
fn with_created(times: FileTimes, created: Option<SystemTime>) -> FileTimes {
    use std::os::macos::fs::FileTimesExt;
    match created {
        Some(t) => times.set_created(t),
        None => times,
    }
}

#[cfg(not(target_os = "macos"))]
fn with_created(times: FileTimes, _created: Option<SystemTime>) -> FileTimes {
    // Linux has no call that sets a file's birth time.
    times
}

/// BSD file flags that stop a file being replaced: user and system immutable ("Locked" in the
/// Finder is `UF_IMMUTABLE`) and append-only (sys/stat.h).
#[cfg(target_os = "macos")]
const LOCK_FLAGS: u32 = 0x0000_0002 | 0x0000_0004 | 0x0002_0000 | 0x0004_0000;

/// Whether the file or folder is locked (macOS `chflags uchg`/`schg`/`uappnd`/`sappnd`).
#[cfg(target_os = "macos")]
pub(crate) fn is_locked(meta: &Metadata) -> bool {
    use std::os::macos::fs::MetadataExt as _;
    meta.st_flags() & LOCK_FLAGS != 0
}

/// Whether the file or folder is locked; Linux has no Finder lock (an immutable inode makes
/// the write fail instead).
#[cfg(not(target_os = "macos"))]
pub(crate) fn is_locked(_meta: &Metadata) -> bool {
    false
}

/// Whether the file at `path` has an access control list, which a file renamed over it would
/// not have (macOS: `acl_get_link_np` through `exacl`; an ACL that only mirrors the mode does
/// not exist there).
///
/// # Errors
/// [`sc_core::Error::Io`] naming `path` when the list cannot be read.
#[cfg(target_os = "macos")]
pub(crate) fn has_acl(path: &Path) -> Result<bool> {
    exacl::getfacl(path, exacl::AclOption::SYMLINK_ACL)
        .map(|entries| !entries.is_empty())
        .map_err(|e| io_err(path, e))
}

/// Whether the file at `path` has an access control list beyond its mode (Linux: the POSIX
/// ACL extended attribute exists only then).
///
/// # Errors
/// Never: a volume without ACL support has none.
#[cfg(not(target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn has_acl(path: &Path) -> Result<bool> {
    Ok(matches!(
        read_xattr(path, OsStr::new("system.posix_acl_access")),
        Ok(Some(_))
    ))
}

#[cfg(test)]
mod tests;
