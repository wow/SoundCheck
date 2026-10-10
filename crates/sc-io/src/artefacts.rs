//! Where a batch's artefacts (the rekordbox XML, [`crate::rekordbox`], and the grid report,
//! [`crate::report`]) go and how they are written. An export to a folder puts them in that
//! folder; an export in place has no folder of its own, so each batch gets a new one under
//! `~/Music/SoundCheck/exports/` named by its local date and time (`2026-10-10 21.04.05`; a
//! second batch in the same second gets ` (2)`). Each file is written to a hidden temp file in
//! the same folder, synced and renamed over any older one, so a reader never sees half of it.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sc_core::{Error, Result};

use crate::txn::fsx::{io_err, remove_if_exists, sync_dir, sync_file, temp_name};

/// Environment variable that overrides the default exports root (used by tests).
pub const EXPORTS_ROOT_ENV: &str = "SC_EXPORTS_ROOT";

/// Most batches one second may name before [`new_batch_folder`] gives up.
const MAX_SAME_SECOND: u32 = 1000;

/// The folder in-place batches put their artefacts under: `$SC_EXPORTS_ROOT` when set, else
/// `~/Music/SoundCheck/exports`.
///
/// # Errors
/// [`Error::Internal`] when the system names no home folder.
pub fn default_exports_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os(EXPORTS_ROOT_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    let dirs = directories::UserDirs::new()
        .ok_or_else(|| Error::Internal("no home folder to keep exports in".into()))?;
    let music = dirs
        .audio_dir()
        .map_or_else(|| dirs.home_dir().join("Music"), Path::to_path_buf);
    Ok(music.join("SoundCheck").join("exports"))
}

/// Creates a new folder for one batch's artefacts under `root` (created if needed), named by
/// `now` in the system's time zone, `yyyy-mm-dd hh.mm.ss` (UTC when the system names no zone),
/// with ` (2)`, ` (3)`, ... when a folder of that name exists.
///
/// # Errors
/// [`Error::Io`] naming the folder that could not be created.
pub fn new_batch_folder(root: &Path, now: SystemTime) -> Result<PathBuf> {
    std::fs::create_dir_all(root).map_err(|e| io_err(root, e))?;
    let name = local_date_time(now);
    for n in 1..=MAX_SAME_SECOND {
        let dir = if n == 1 {
            root.join(&name)
        } else {
            root.join(format!("{name} ({n})"))
        };
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_err(&dir, e)),
        }
    }
    Err(io_err(
        &root.join(&name),
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "too many batch folders for one second",
        ),
    ))
}

/// `t` in the system's time zone as `yyyy-mm-dd hh.mm.ss` (Finder shows `:` as `/`, so dots).
pub(crate) fn local_date_time(t: SystemTime) -> String {
    let zoned = jiff::Timestamp::try_from(t)
        .ok()
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()));
    match zoned {
        Some(z) => format!(
            "{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
            z.year(),
            z.month(),
            z.day(),
            z.hour(),
            z.minute(),
            z.second()
        ),
        None => "1970-01-01 00.00.00".to_owned(),
    }
}

/// Writes `bytes` to `path`, replacing any file there atomically: a hidden temp file next to
/// it, synced, renamed over it, the folder synced.
///
/// # Errors
/// [`Error::Io`] naming `path` or its temp file; the temp file is removed.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| Error::InvalidArgument(format!("{} names no file", path.display())))?;
    let id = format!("{}-{}", std::process::id(), unique());
    let temp = dir.join(temp_name(name, &id));
    let written = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| io_err(&temp, e))?;
        f.write_all(bytes).map_err(|e| io_err(&temp, e))?;
        sync_file(&f, &temp)?;
        drop(f);
        std::fs::rename(&temp, path).map_err(|e| io_err(path, e))
    })();
    if let Err(e) = written {
        let _ = remove_if_exists(&temp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(())
}

/// Bytes read from an existing artefact to tell whether SoundCheck wrote it.
const HEAD_BYTES: usize = 512;

/// Writes `bytes` to `path` like [`write_file`], but replaces only a file SoundCheck wrote:
/// a file there whose first bytes `is_ours` rejects (an audio file, someone else's XML), or
/// anything but a regular file, is left as it is.
///
/// # Errors
/// [`Error::AlreadyExists`] naming `path` when something not ours is there; [`Error::Io`] as
/// [`write_file`].
pub fn write_artefact(path: &Path, bytes: &[u8], is_ours: fn(&[u8]) -> bool) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let ours = meta.is_file() && {
                use std::io::Read;
                let mut head = Vec::with_capacity(HEAD_BYTES);
                std::fs::File::open(path)
                    .and_then(|f| f.take(HEAD_BYTES as u64).read_to_end(&mut head))
                    .map_err(|e| io_err(path, e))?;
                is_ours(&head)
            };
            if !ours {
                return Err(Error::AlreadyExists {
                    path: path.to_path_buf(),
                });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err(path, e)),
    }
    write_file(path, bytes)
}

/// A number no other call in this process got.
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests;
