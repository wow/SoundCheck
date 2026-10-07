//! File metadata a rename does not carry, and the checks that decide whether a file may be
//! replaced in place.
//!
//! A new file renamed over an old one has the new file's metadata: no extended attributes (on
//! macOS: Finder tags and colour label, Finder comment, "Where from", quarantine, the resource
//! fork), a new creation date, the default mode. [`snapshot`] reads them from the original
//! before the rename and [`restore`] writes them onto the new file after it: every extended
//! attribute byte for byte, the creation date (macOS `setattrlist(ATTR_CMN_CRTIME)` through
//! std), the modification time when asked, and the mode last. Access control lists are not
//! carried; files that have one are refused in place instead ([`has_acl`]).

use std::ffi::OsString;
use std::fs::{FileTimes, Metadata, OpenOptions};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::SystemTime;

use sc_core::Result;

use super::fsx::io_err;

/// The metadata of a file that SoundCheck restores after replacing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileMeta {
    /// Length, bytes (to notice a change, not restored).
    pub len: u64,
    /// Permission bits (`st_mode & 0o7777`).
    pub mode: u32,
    /// Modification time.
    pub modified: SystemTime,
    /// Creation (birth) time, where the system reports one.
    pub created: Option<SystemTime>,
    /// Every extended attribute, by name in byte order.
    pub xattrs: Vec<(OsString, Vec<u8>)>,
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
        match xattr::get(path, &name) {
            Ok(Some(value)) => xattrs.push((name, value)),
            // Removed between the list and the read, or not readable by this user.
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(path = %path.display(), name = %name.to_string_lossy(), error = %e, "extended attribute not readable");
            }
        }
    }
    Ok(FileMeta {
        len: meta.len(),
        mode: meta.mode() & 0o7777,
        modified,
        created: meta.created().ok(),
        xattrs,
    })
}

/// Writes `meta` onto the file at `path`: every extended attribute, then the creation date and
/// (with `keep_mtime`) the modification time, then the mode. Returns the names of extended
/// attributes the volume or the system refused (for example a protected `com.apple.*` name);
/// they are logged and the rest is restored.
///
/// # Errors
/// [`sc_core::Error::Io`] naming `path` when the times or the mode cannot be set.
pub(crate) fn restore(path: &Path, meta: &FileMeta, keep_mtime: bool) -> Result<Vec<String>> {
    let mut refused = Vec::new();
    for (name, value) in &meta.xattrs {
        let same = matches!(xattr::get(path, name), Ok(Some(v)) if v == *value);
        if same {
            continue;
        }
        if let Err(e) = xattr::set(path, name, value) {
            let name = name.to_string_lossy().into_owned();
            tracing::warn!(path = %path.display(), name, error = %e, "extended attribute not restored");
            refused.push(name);
        }
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
    Ok(refused)
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
        xattr::get(path, "system.posix_acl_access"),
        Ok(Some(_))
    ))
}
