//! Saved grid edits: one small JSON file per audio file, holding the user's overrides, the grid
//! they gave and whether the user confirmed it.
//!
//! Edits are user work, so they live under Application Support, not with the analysis cache
//! that macOS may purge: `~/Library/Application Support/app.soundcheck.desktop/grid-edits` on
//! macOS (`SC_EDITS_DIR` overrides it). The file name is the BLAKE3 hash of the NFC-normalised
//! path and the audio identity, so a path keeps one edit per version of its audio: exporting a
//! file in place saves the edit of the new audio next to the original's, and putting the original
//! back (undo) finds the original's edit again. Files written before edits were kept per audio
//! are named by the path alone; they are still read, for the audio they were made on, and
//! replaced by the next save of that audio.
//!
//! An edit belongs to the audio it was made on, identified by its decoded length, sample rate
//! and integrated loudness: DJ apps rewrite tags and change the file's size and modification
//! time without touching the audio, and the edit must survive that. It also carries the DJ-app
//! BPM range it was made under, because the solver's octave choice depends on it, and the grid
//! it gave, so a confirmation counts only while the edit still gives that grid.

use std::io::Write;
use std::path::{Path, PathBuf};

use sc_core::analysis::{GridEdit, Meter};
use sc_core::{Bpm, Error, Lufs, Result, SampleIndex};
use serde::{Deserialize, Serialize};

/// Schema of a saved edit; bumped when its shape changes.
pub const EDIT_SCHEMA: u32 = 1;

/// Environment variable that overrides the edits directory (tests, CI).
pub const EDITS_DIR_ENV: &str = "SC_EDITS_DIR";

/// The audio an edit was made on: what tag edits leave alone and a different master changes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioIdentity {
    /// Playable frames after encoder delay and padding.
    pub frames: u64,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Integrated loudness; `None` for silence.
    pub integrated: Option<Lufs>,
}

/// The grid an edit gave when it was saved: what the user saw, heard and confirmed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridPin {
    /// Tempo at the meter's unit.
    pub bpm: Bpm,
    /// Bar 1 at the native rate.
    pub anchor: SampleIndex,
    /// Pulse count and grouping.
    pub meter: Meter,
}

/// One file's saved edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEdit {
    /// [`EDIT_SCHEMA`].
    pub schema: u32,
    /// Absolute path, NFC-normalised.
    pub path: String,
    /// The audio the edit was made on.
    pub audio: AudioIdentity,
    /// The DJ-app BPM range the edit was made under.
    pub bpm_range: (Bpm, Bpm),
    /// The overrides; empty when the user confirmed the analysed grid as it was.
    pub edit: GridEdit,
    /// The grid the edit gave; `None` when the file has no grid.
    pub grid: Option<GridPin>,
    /// The user confirmed `grid` by ear.
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

    /// The edit file of the audio `audio` at an NFC path.
    #[must_use]
    pub fn entry_path(&self, nfc_path: &str, audio: &AudioIdentity) -> PathBuf {
        let mut h = blake3::Hasher::new();
        h.update(nfc_path.as_bytes());
        h.update(&[0]);
        h.update(&audio.frames.to_le_bytes());
        h.update(&audio.sample_rate.to_le_bytes());
        match audio.integrated {
            Some(l) => h.update(&[1]).update(&l.0.to_bits().to_le_bytes()),
            None => h.update(&[0]),
        };
        self.dir.join(format!("{}.json", h.finalize().to_hex()))
    }

    /// The edit file of an NFC path as named before edits were kept per audio.
    #[must_use]
    pub fn legacy_entry_path(&self, nfc_path: &str) -> PathBuf {
        let name = blake3::hash(nfc_path.as_bytes()).to_hex();
        self.dir.join(format!("{name}.json"))
    }

    /// The edit saved for the audio `audio` of the file at `nfc_path`; a missing, unreadable or
    /// other-schema edit is `None` (an unreadable one is logged, since it held user work).
    #[must_use]
    pub fn get(&self, nfc_path: &str, audio: &AudioIdentity) -> Option<SavedEdit> {
        Self::read(&self.entry_path(nfc_path, audio), nfc_path, audio)
            .or_else(|| Self::read(&self.legacy_entry_path(nfc_path), nfc_path, audio))
    }

    fn read(file: &Path, nfc_path: &str, audio: &AudioIdentity) -> Option<SavedEdit> {
        let bytes = std::fs::read(file).ok()?;
        match serde_json::from_slice::<SavedEdit>(&bytes) {
            Ok(saved)
                if saved.schema == EDIT_SCHEMA
                    && saved.path == nfc_path
                    && saved.audio == *audio =>
            {
                Some(saved)
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(file = %file.display(), error = %e, "unreadable grid edit ignored");
                None
            }
        }
    }

    /// Saves `saved`, replacing any edit of the same file: written to a temp file, flushed to
    /// disk, then renamed over the old one, so a crash leaves either edit whole.
    ///
    /// # Errors
    /// [`Error::Io`] when the directory cannot be created or the file written.
    pub fn put(&self, saved: &SavedEdit) -> Result<()> {
        let target = self.entry_path(&saved.path, &saved.audio);
        let io = |source| Error::Io {
            path: target.clone(),
            source,
        };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let json = serde_json::to_vec_pretty(saved)
            .map_err(|e| io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let temp = target.with_extension(format!("tmp-{}", std::process::id()));
        let written = std::fs::File::create(&temp)
            .and_then(|mut f| f.write_all(&json).and_then(|()| f.sync_all()))
            .and_then(|()| std::fs::rename(&temp, &target));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(io(e));
        }
        self.remove_legacy(&saved.path, &saved.audio)
    }

    /// Removes the edit of the audio `audio` of the file at `nfc_path` (the analysed grid applies
    /// again); a missing edit is not an error. Edits of other audio at the path stay.
    ///
    /// # Errors
    /// [`Error::Io`] when the file exists but cannot be removed.
    pub fn remove(&self, nfc_path: &str, audio: &AudioIdentity) -> Result<()> {
        remove_file(&self.entry_path(nfc_path, audio))?;
        self.remove_legacy(nfc_path, audio)
    }

    /// Removes the file named by the path alone when it holds the edit of `audio`.
    fn remove_legacy(&self, nfc_path: &str, audio: &AudioIdentity) -> Result<()> {
        let legacy = self.legacy_entry_path(nfc_path);
        if Self::read(&legacy, nfc_path, audio).is_some() {
            remove_file(&legacy)?;
        }
        Ok(())
    }
}

/// Removes `path`; a missing file is not an error.
fn remove_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests;
