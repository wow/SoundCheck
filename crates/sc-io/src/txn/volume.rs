//! Which volume a path lies on: its root, a name for the backup tree, free space, and whether
//! it is a rekordbox USB export.
//!
//! The rule that names a volume (it names the folder its backups go under, so it must not
//! change from one run to the next):
//!
//! - **macOS**: the mount point comes from `statfs(2)` (`f_mntonname`). A volume mounted at
//!   `/Volumes/<name>` is called `<name>`. The startup disk (mounted at `/`, with its data
//!   volume at `/System/Volumes/Data` reached through the system's firmlinks, so a path such as
//!   `/Users/me/Music/a.wav` lies on it) is called by the name of the `/Volumes` entry that is a
//!   symbolic link to `/` (the disk's name in the Finder, "Macintosh HD" by default), or
//!   "Startup Disk" when there is none. Any other mount point is called by its last component.
//!   Paths on the startup disk are taken relative to `/`, others relative to their mount point.
//! - **Linux**: the mount point is the highest ancestor on the same device (`st_dev`); `/` is
//!   called "Root", any other mount point by its last component.
//!
//! [`VolumeProvider`] is the seam tests use to simulate a rekordbox USB export or a full disk.

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use sc_core::Result;

use super::fsx::io_err;

/// The folder rekordbox creates at the root of a USB export.
pub const REKORDBOX_EXPORT_FOLDER: &str = "PIONEER";

/// Name given to the startup disk when no `/Volumes` entry links to `/`.
pub const STARTUP_DISK_NAME: &str = "Startup Disk";

/// A volume as the transaction sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// The folder paths on this volume are taken relative to (its mount point; `/` for the
    /// startup disk on macOS).
    pub root: PathBuf,
    /// The name its backups are stored under (see the module documentation for the rule).
    pub name: String,
    /// Identifies the volume: two paths with the same id share free space (`st_dev`).
    pub id: u64,
    /// Bytes this user may still write.
    pub free_bytes: u64,
}

impl Volume {
    /// Whether the volume is a rekordbox USB export: a `PIONEER` folder at its root.
    #[must_use]
    pub fn is_rekordbox_export(&self) -> bool {
        self.root.join(REKORDBOX_EXPORT_FOLDER).is_dir()
    }

    /// `path` relative to the volume's root (`None` when it does not lie under it).
    #[must_use]
    pub fn relative<'p>(&self, path: &'p Path) -> Option<&'p Path> {
        path.strip_prefix(&self.root).ok()
    }
}

/// Where volume facts come from. [`SystemVolumes`] asks the operating system; tests provide
/// their own to simulate volumes.
pub trait VolumeProvider: Send + Sync {
    /// The volume holding `path`, an existing absolute path.
    ///
    /// # Errors
    /// [`sc_core::Error::Io`] naming `path` when the system cannot say.
    fn volume_of(&self, path: &Path) -> Result<Volume>;
}

/// The operating system's view of volumes.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemVolumes;

impl VolumeProvider for SystemVolumes {
    fn volume_of(&self, path: &Path) -> Result<Volume> {
        let meta = std::fs::metadata(path).map_err(|e| io_err(path, e))?;
        let (mount, free_bytes) = mount_and_free(path)?;
        let root = root_for(&mount, path);
        Ok(Volume {
            name: volume_name(&mount, startup_disk_name),
            root,
            id: meta.dev(),
            free_bytes,
        })
    }
}

/// The macOS mount point of the startup disk's data volume.
const DATA_VOLUME: &str = "/System/Volumes/Data";

/// The folder paths are taken relative to: `/` for a path on the startup disk's data volume
/// reached through a firmlink (not under [`DATA_VOLUME`] itself), else the mount point.
pub(crate) fn root_for(mount: &Path, path: &Path) -> PathBuf {
    if mount == Path::new(DATA_VOLUME) && !path.starts_with(mount) {
        PathBuf::from("/")
    } else {
        mount.to_path_buf()
    }
}

/// The name of the volume mounted at `mount` (the rule in the module documentation);
/// `startup` gives the startup disk's name when the system has one.
pub(crate) fn volume_name(mount: &Path, startup: impl Fn() -> Option<String>) -> String {
    let parts: Vec<Component<'_>> = mount.components().collect();
    if let [
        Component::RootDir,
        Component::Normal(v),
        Component::Normal(name),
    ] = parts.as_slice()
        && *v == "Volumes"
    {
        return clean(&name.to_string_lossy());
    }
    if mount == Path::new("/") || mount == Path::new(DATA_VOLUME) {
        if cfg!(target_os = "macos") {
            return startup().map_or_else(|| STARTUP_DISK_NAME.to_owned(), |n| clean(&n));
        }
        return "Root".to_owned();
    }
    mount
        .file_name()
        .map_or_else(|| "Volume".to_owned(), |n| clean(&n.to_string_lossy()))
}

/// A volume name usable as one folder name.
fn clean(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c == '/' { ':' } else { c })
        .collect();
    match name.as_str() {
        "" | "." | ".." => "Volume".to_owned(),
        _ => name,
    }
}

/// The name of the `/Volumes` entry that is a symbolic link to `/` (the first in byte order
/// when there are several).
fn startup_disk_name() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/Volumes")
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|e| std::fs::read_link(e.path()).is_ok_and(|t| t == Path::new("/")))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.into_iter().next()
}

/// A lasting identity of the volume holding the existing `path`, the same at every mount and
/// different for another volume mounted at the same place (a second USB stick with the same
/// name): on macOS `startup` for the startup disk (it cannot change while the system runs, and
/// its device number may change between boots), else the volume UUID `diskutil info` reports
/// for the mount point (APFS and HFS+ UUIDs, the exFAT and FAT serial numbers); on Linux the
/// `statfs(2)` file system id (derived from the file system UUID on ext4 and btrfs). `None`
/// when the system cannot say.
///
/// `diskutil` takes about 0.2 s, so with `cached` an answer is reused for [`IDENTITY_TTL`] for
/// the same mount point, device, size and root folder; recovery asks afresh.
#[must_use]
pub fn volume_identity(path: &Path, cached: bool) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Mutex;
        use std::time::Instant;
        type Key = (
            PathBuf,
            u64,
            u64,
            u64,
            Option<std::time::SystemTime>,
            String,
        );
        static CACHE: Mutex<Vec<(Key, Instant, String)>> = Mutex::new(Vec::new());
        let s = rustix::fs::statfs(path).ok()?;
        let (mount, _) = mount_and_free(path).ok()?;
        if mount == Path::new("/") || mount == Path::new(DATA_VOLUME) {
            return Some("startup".to_owned());
        }
        let root = std::fs::metadata(&mount).ok()?;
        let key: Key = (
            mount.clone(),
            root.dev(),
            s.f_blocks,
            root.ino(),
            root.created().ok(),
            // Two same-sized FAT sticks swapped within the reuse window share every other part.
            format!("{:?}", s.f_fsid),
        );
        let mut cache = CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|(_, at, _)| at.elapsed() < IDENTITY_TTL);
        if cached && let Some((_, _, id)) = cache.iter().find(|(k, _, _)| *k == key) {
            return Some(id.clone());
        }
        drop(cache);
        let mut command = std::process::Command::new("/usr/sbin/diskutil");
        command.args(["info", "-plist"]).arg(&mount);
        let stdout = run_with_deadline(command, DISKUTIL_DEADLINE)?;
        let id = plist_string(&String::from_utf8_lossy(&stdout), "VolumeUUID")?;
        let mut cache = CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.push((key, Instant::now(), id.clone()));
        Some(id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cached;
        let s = rustix::fs::statfs(path).ok()?;
        Some(format!("fsid:{:?}", s.f_fsid))
    }
}

/// The longest [`volume_identity`] waits for `diskutil`, which can stall on an unresponsive
/// drive; past it the volume has no identity (a write records none, recovery stays pending).
pub const DISKUTIL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs `command` and returns its standard output when it exits successfully within
/// `deadline`; kills it and returns `None` when it does not.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only macOS asks diskutil
pub(crate) fn run_with_deadline(
    mut command: std::process::Command,
    deadline: std::time::Duration,
) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::time::Instant;
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // Read on another thread so a full pipe cannot stall the child while we wait.
    let mut pipe = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        pipe.read_to_end(&mut out).map(|_| out)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().ok()?.ok()?;
    status
        .filter(std::process::ExitStatus::success)
        .map(|_| out)
}

/// How long [`volume_identity`] reuses an answer.
pub const IDENTITY_TTL: std::time::Duration = std::time::Duration::from_secs(10);

/// The `<string>` value following `<key>key</key>` in a property list.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only macOS reads diskutil's plist
pub(crate) fn plist_string(plist: &str, key: &str) -> Option<String> {
    let after = &plist[plist.find(&format!("<key>{key}</key>"))?..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")? + start;
    let value = after[start..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Mount point and bytes free for this user (`statfs(2)`: `f_mntonname`, `f_bavail` x
/// `f_bsize`).
#[cfg(target_os = "macos")]
fn mount_and_free(path: &Path) -> Result<(PathBuf, u64)> {
    use std::os::unix::ffi::OsStringExt;
    let s = rustix::fs::statfs(path).map_err(|e| io_err(path, e.into()))?;
    let bytes: Vec<u8> = s
        .f_mntonname
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| c.cast_unsigned())
        .collect();
    let mount = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    Ok((mount, s.f_bavail.saturating_mul(u64::from(s.f_bsize))))
}

/// Mount point (the highest ancestor on the same device) and bytes free for this user
/// (`statvfs(2)`: `f_bavail` x `f_frsize`).
#[cfg(not(target_os = "macos"))]
fn mount_and_free(path: &Path) -> Result<(PathBuf, u64)> {
    let s = rustix::fs::statvfs(path).map_err(|e| io_err(path, e.into()))?;
    let dev = std::fs::metadata(path).map_err(|e| io_err(path, e))?.dev();
    let mut mount = path.to_path_buf();
    while let Some(parent) = mount.parent() {
        match std::fs::metadata(parent) {
            Ok(m) if m.dev() == dev => mount = parent.to_path_buf(),
            _ => break,
        }
    }
    Ok((mount, s.f_bavail.saturating_mul(s.f_frsize)))
}

#[cfg(test)]
mod tests;
