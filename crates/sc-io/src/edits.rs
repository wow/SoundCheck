//! Saved grid edits: one small JSON file per audio file, holding the user's overrides and
//! whether they confirmed the grid.
//!
//! Edits are user work, so they live under Application Support, not with the analysis cache
//! that macOS may purge: `~/Library/Application Support/app.soundcheck.desktop/grid-edits` on
//! macOS (`SC_EDITS_DIR` overrides it). The file name is the BLAKE3 hash of the NFC-normalised
//! path, as for the cache. An edit applies only while the file keeps the size and modification
//! time it had when the edit was saved; a changed file gets the analysis alone again.
//!
//! Each edit carries the DJ-app BPM range it was made under, because the solver's octave choice
//! depends on it: solving the file's evidence with that range and the overrides gives back the
//! grid the user saw, even after the range setting changes.

use std::path::{Path, PathBuf};

use sc_core::analysis::GridEdit;
use sc_core::{Bpm, Error, Result};
use serde::{Deserialize, Serialize};

/// Schema of a saved edit; bumped when its shape changes.
pub const EDIT_SCHEMA: u32 = 1;

/// Environment variable that overrides the edits directory (tests, CI).
pub const EDITS_DIR_ENV: &str = "SC_EDITS_DIR";

/// One file's saved edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEdit {
    /// [`EDIT_SCHEMA`].
    pub schema: u32,
    /// Absolute path, NFC-normalised.
    pub path: String,
    /// File size in bytes when the edit was saved.
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch when the edit was saved.
    pub mtime_ns: i64,
    /// The DJ-app BPM range the edit was made under.
    pub bpm_range: (Bpm, Bpm),
    /// The overrides; empty when the user confirmed the analysed grid as it was.
    pub edit: GridEdit,
    /// The user confirmed the grid by ear.
    pub confirmed: bool,
}

/// A directory of saved edits.
#[derive(Debug, Clone)]
pub struct EditStore {
    dir: PathBuf,
}

impl EditStore {
    /// The platform directory for saved edits, or `SC_EDITS_DIR` when set.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when the platform has no data directory for this user.
    pub fn default_dir() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os(EDITS_DIR_ENV) {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("app", "soundcheck", "desktop")
            .map(|dirs| dirs.data_dir().join("grid-edits"))
            .ok_or_else(|| Error::InvalidArgument("no data directory for this user".into()))
    }

    /// A store in `dir`; the directory is created on the first save.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Where edits live.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The edit file for an NFC path.
    #[must_use]
    pub fn entry_path(&self, nfc_path: &str) -> PathBuf {
        let name = blake3::hash(nfc_path.as_bytes()).to_hex();
        self.dir.join(format!("{name}.json"))
    }

    /// The edit saved for the file at `nfc_path` while it has `size` bytes and modification
    /// time `mtime_ns`; a missing, unreadable, other-schema or stale edit is `None`.
    #[must_use]
    pub fn get(&self, nfc_path: &str, size: u64, mtime_ns: i64) -> Option<SavedEdit> {
        let bytes = std::fs::read(self.entry_path(nfc_path)).ok()?;
        let saved: SavedEdit = serde_json::from_slice(&bytes).ok()?;
        (saved.schema == EDIT_SCHEMA
            && saved.path == nfc_path
            && saved.size == size
            && saved.mtime_ns == mtime_ns)
            .then_some(saved)
    }

    /// Saves `saved`, replacing any edit of the same file, atomically (temp file, then rename).
    ///
    /// # Errors
    /// [`Error::Io`] when the directory cannot be created or the file written.
    pub fn put(&self, saved: &SavedEdit) -> Result<()> {
        let target = self.entry_path(&saved.path);
        let io = |source| Error::Io {
            path: target.clone(),
            source,
        };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let json = serde_json::to_vec_pretty(saved)
            .map_err(|e| io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let temp = target.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temp, json).map_err(io)?;
        std::fs::rename(&temp, &target).map_err(io)?;
        Ok(())
    }

    /// Removes the edit of the file at `nfc_path` (the analysed grid applies again); a missing
    /// edit is not an error.
    ///
    /// # Errors
    /// [`Error::Io`] when the file exists but cannot be removed.
    pub fn remove(&self, nfc_path: &str) -> Result<()> {
        let path = self.entry_path(nfc_path);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::SampleIndex;

    fn saved(path: &str) -> SavedEdit {
        SavedEdit {
            schema: EDIT_SCHEMA,
            path: path.into(),
            size: 1000,
            mtime_ns: 42,
            bpm_range: (Bpm(70.0), Bpm(180.0)),
            edit: GridEdit {
                octave: -1,
                anchor: Some(SampleIndex(22_050)),
                ..GridEdit::default()
            },
            confirmed: true,
        }
    }

    #[test]
    fn an_edit_is_served_only_while_the_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = EditStore::open(dir.path().join("edits"));
        assert!(store.get("/music/a.flac", 1000, 42).is_none());
        store.put(&saved("/music/a.flac")).unwrap();
        assert_eq!(
            store.get("/music/a.flac", 1000, 42),
            Some(saved("/music/a.flac"))
        );
        assert!(store.get("/music/a.flac", 1001, 42).is_none(), "size");
        assert!(store.get("/music/a.flac", 1000, 43).is_none(), "mtime");
        assert!(store.get("/music/b.flac", 1000, 42).is_none(), "other file");
        store.remove("/music/a.flac").unwrap();
        store.remove("/music/a.flac").unwrap();
        assert!(store.get("/music/a.flac", 1000, 42).is_none());
    }

    #[test]
    fn a_newer_save_replaces_the_edit_and_no_temp_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let store = EditStore::open(dir.path());
        store.put(&saved("/music/a.flac")).unwrap();
        let newer = SavedEdit {
            size: 2000,
            confirmed: false,
            ..saved("/music/a.flac")
        };
        store.put(&newer).unwrap();
        assert_eq!(store.get("/music/a.flac", 2000, 42), Some(newer));
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn a_saved_edit_is_readable_json() {
        let json = serde_json::to_string(&saved("/music/a.flac")).unwrap();
        assert!(json.contains("\"bpmRange\":[70.0,180.0]"), "{json}");
        assert!(json.contains("\"mtimeNs\":42"), "{json}");
        assert!(json.contains("\"octave\":-1"), "{json}");
    }
}
