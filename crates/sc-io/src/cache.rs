//! The analysis cache: one JSON file per audio file under the user's cache directory.
//!
//! An entry is valid only when the file's inode and status change time, its size and
//! modification time, the analysis settings and the SoundCheck version all match, so a changed
//! file, changed settings or a new release never serve a stale record. The inode and change
//! time matter because SoundCheck itself writes files at the same length with their
//! modification time put back: such a write replaces the file by a new one (a new inode), and
//! the change time cannot be set back. The device is left out: it changes when another disk
//! mounts first, and the path already names the volume. On FAT and exFAT the change time is the
//! modification time and inodes are reusable slot numbers, so the writers also remove a file's
//! entry ([`Cache::remove`]) before and after every change they make. The file name is the BLAKE3 hash of the NFC-normalised absolute path:
//! macOS stores names in NFD while rekordbox and most tools use NFC, so every path comparison in
//! SoundCheck normalises first. Analysis never writes next to the music; the cache lives under
//! `~/Library/Caches/app.soundcheck.desktop/analysis` on macOS (`SC_CACHE_DIR` overrides it).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use sc_core::analysis::{AnalysisRecord, AnalysisSettings};
use sc_core::{Error, Result};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Schema of the cache entry wrapper; bumped when the wrapper changes shape. Version 2 added the
/// file identity to the key; entries of version 1 miss (the file is analysed once more).
pub const CACHE_SCHEMA: u32 = 2;

/// Environment variable that overrides the cache directory (tests, CI).
pub const CACHE_DIR_ENV: &str = "SC_CACHE_DIR";

/// What must match for a cached record to be served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheKey {
    /// File size in bytes.
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
    /// Inode: a file replaced by a rename has a new one.
    pub ino: u64,
    /// Status change time in nanoseconds since the Unix epoch, which no write can set back.
    pub ctime_ns: i64,
    /// [`AnalysisSettings::settings_hash`].
    pub settings_hash: u64,
    /// SoundCheck version that wrote the entry.
    pub version: String,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    schema: u32,
    key: CacheKey,
    record: AnalysisRecord,
}

/// A cache directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// The platform cache directory for SoundCheck, or `SC_CACHE_DIR` when set.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when the platform has no cache directory for this user.
    pub fn default_dir() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os(CACHE_DIR_ENV) {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("app", "soundcheck", "desktop")
            .map(|dirs| dirs.cache_dir().join("analysis"))
            .ok_or_else(|| Error::InvalidArgument("no cache directory for this user".into()))
    }

    /// A cache in `dir`; the directory is created on the first write.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Where entries live.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The NFC-normalised absolute path and the key of `path` under `settings`.
    ///
    /// # Errors
    /// [`Error::Io`] when the file cannot be stat'ed.
    pub fn key_for(path: &Path, settings: &AnalysisSettings) -> Result<(String, CacheKey)> {
        let io = |source| Error::Io {
            path: path.into(),
            source,
        };
        let absolute = std::path::absolute(path).map_err(io)?;
        let meta = std::fs::metadata(&absolute).map_err(io)?;
        let mtime = meta.modified().map_err(io)?;
        let mtime_ns = match mtime.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
        };
        let key = CacheKey {
            size: meta.len(),
            mtime_ns,
            ino: meta.ino(),
            ctime_ns: meta
                .ctime()
                .saturating_mul(1_000_000_000)
                .saturating_add(meta.ctime_nsec()),
            settings_hash: settings.settings_hash(),
            version: sc_core::VERSION.into(),
        };
        Ok((nfc(&absolute), key))
    }

    /// The entry file for an NFC path.
    #[must_use]
    pub fn entry_path(&self, nfc_path: &str) -> PathBuf {
        let name = blake3::hash(nfc_path.as_bytes()).to_hex();
        self.dir.join(format!("{name}.json"))
    }

    /// The cached record for `nfc_path` when its key matches; a missing, unreadable or stale
    /// entry is `None`.
    #[must_use]
    pub fn get(&self, nfc_path: &str, key: &CacheKey) -> Option<AnalysisRecord> {
        let bytes = std::fs::read(self.entry_path(nfc_path)).ok()?;
        let entry: CacheEntry = serde_json::from_slice(&bytes)
            .ok()
            .or_else(|| older_entry(&bytes))?;
        (entry.schema == CACHE_SCHEMA && &entry.key == key).then_some(entry.record)
    }

    /// Writes `record` for `nfc_path`, atomically (temp file, then rename).
    ///
    /// # Errors
    /// [`Error::Io`] when the directory cannot be created or the file written.
    pub fn put(&self, nfc_path: &str, key: &CacheKey, record: &AnalysisRecord) -> Result<()> {
        let target = self.entry_path(nfc_path);
        let io = |source| Error::Io {
            path: target.clone(),
            source,
        };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let entry = CacheEntry {
            schema: CACHE_SCHEMA,
            key: key.clone(),
            record: record.clone(),
        };
        let json = serde_json::to_vec(&entry)
            .map_err(|e| io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let temp = target.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, json).map_err(io)?;
        std::fs::rename(&temp, &target).map_err(io)?;
        Ok(())
    }

    /// Removes the entry of the file at `path` (made absolute and NFC-normalised as
    /// [`Cache::key_for`] does); whether there was one.
    ///
    /// # Errors
    /// [`Error::Io`] when the path cannot be made absolute or the entry cannot be removed.
    pub fn remove(&self, path: &Path) -> Result<bool> {
        let absolute = std::path::absolute(path).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })?;
        let entry = self.entry_path(&nfc(&absolute));
        match std::fs::remove_file(&entry) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(Error::Io {
                path: entry,
                source,
            }),
        }
    }

    /// Removes every entry; returns how many were removed.
    ///
    /// # Errors
    /// [`Error::Io`] when an entry cannot be removed (a missing directory is not an error).
    pub fn clear(&self) -> Result<usize> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Ok(0);
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                std::fs::remove_file(&path).map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// An entry whose record is older than the current schema. Schema-1 records stored their grid
/// evidence in another shape: they still serve the table without it, and the grid view analyses
/// such a file again before refitting it.
fn older_entry(bytes: &[u8]) -> Option<CacheEntry> {
    let mut entry: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let record = entry.get_mut("record")?;
    if record.get("schema")?.as_u64()? != 1 {
        return None;
    }
    record["evidence"] = serde_json::Value::Null;
    serde_json::from_value(entry).ok()
}

/// The path as an NFC-normalised string (lossy for non-UTF-8 bytes).
#[must_use]
pub fn nfc(path: &Path) -> String {
    path.to_string_lossy().nfc().collect()
}

#[cfg(test)]
mod tests;
