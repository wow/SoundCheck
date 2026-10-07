//! The app's log file: `tracing` events at `info` and above (or as `RUST_LOG` says), appended
//! to `soundcheck.log` in `~/Library/Logs/app.soundcheck.desktop/` on macOS (where Console.app
//! finds it) and in the local data folder elsewhere. Without it, what the start-up recovery did
//! would go nowhere.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// The app's bundle identifier, which names its log folder.
const APP_ID: &str = "app.soundcheck.desktop";

/// The log file's name.
pub(crate) const LOG_FILE: &str = "soundcheck.log";

/// The folder the log file goes in, when the system names a home folder.
pub(crate) fn log_dir() -> Option<PathBuf> {
    let dirs = directories::BaseDirs::new()?;
    if cfg!(target_os = "macos") {
        Some(dirs.home_dir().join("Library/Logs").join(APP_ID))
    } else {
        Some(dirs.data_local_dir().join(APP_ID).join("logs"))
    }
}

/// Opens `<dir>/soundcheck.log` for appending, creating the folder and the file if needed.
pub(crate) fn open_log(dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(LOG_FILE))
}

/// Sends `tracing` events to the log file; returns its path. Nothing is logged when the file
/// cannot be opened or a subscriber is already set.
pub(crate) fn init() -> Option<PathBuf> {
    let dir = log_dir()?;
    let file = open_log(&dir).ok()?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .try_init()
        .ok()?;
    let path = dir.join(LOG_FILE);
    tracing::info!(version = sc_core::VERSION, log = %path.display(), "SoundCheck started");
    Some(path)
}

#[cfg(test)]
mod tests;
