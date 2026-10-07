//! The transaction journal: `journal.jsonl` in the backup root, one JSON object per line, one
//! line per state change, each line synced before the step it records is relied on.
//!
//! A transaction's lines share its id (`txn`); the first (`planned`) holds what the transaction
//! will do, later ones add what each step learned (the output's hash, the backup's path and
//! hash). [`fold`] turns the lines into one [`Entry`] per transaction. A line that does not
//! parse (a line cut short by a crash) is skipped with a warning.
//!
//! While a transaction runs it holds an exclusive lock on `locks/<txn>.lock` in the backup
//! root ([`TxnLock`]); the lock file is created before the first line and removed after the
//! last, and the operating system drops the lock when the process dies. Recovery only touches
//! transactions whose lock it can take, so it never races a live one.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use sc_core::{Error, Result};

use super::fsx::{io_err, remove_if_exists, sync_file};
use super::sidecar::Record;

/// File name of the journal in the backup root.
pub const JOURNAL_FILE: &str = "journal.jsonl";

/// Folder of the per-transaction lock files in the backup root.
const LOCK_DIR: &str = "locks";

/// A transaction's state, in the order the steps happen; the last three end a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Checked and about to write the temp file.
    Planned,
    /// The temp file is written and synced.
    TempWritten,
    /// The temp file read back as written.
    Verified,
    /// The original is copied to the backup root, synced and hashed (in place only).
    BackedUp,
    /// The temp file was renamed over the target.
    Renamed,
    /// Extended attributes, dates and mode are restored.
    MetadataDone,
    /// Finished, sidecar written.
    Done,
    /// Stopped before the rename; the temp file and backup were removed and the target is
    /// untouched.
    Failed,
    /// Finished or rolled back by [`super::recover`] after a crash.
    Recovered,
}

impl State {
    /// The name used in the journal and by `SC_TEST_CRASH_AFTER_STEP`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::TempWritten => "temp_written",
            Self::Verified => "verified",
            Self::BackedUp => "backed_up",
            Self::Renamed => "renamed",
            Self::MetadataDone => "metadata_done",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Recovered => "recovered",
        }
    }

    /// Whether the transaction is over.
    #[must_use]
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Recovered)
    }
}

/// What a transaction does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxnKind {
    /// Replaces a file by its rendered version, after backing it up.
    InPlace,
    /// Writes the rendered version into another folder; the source is untouched.
    ToFolder,
    /// Puts a backup back in place of the rendered version.
    Undo,
}

impl TxnKind {
    /// The last state before the rename.
    #[must_use]
    pub fn pre_rename(self) -> State {
        match self {
            Self::InPlace => State::BackedUp,
            Self::ToFolder | Self::Undo => State::Verified,
        }
    }
}

/// How recovery ended a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing was renamed: the temp file (and a backup made for it) were removed and the
    /// target holds what it held before.
    RolledBack,
    /// The rename had happened: metadata and sidecar were finished.
    Completed,
}

/// One journal line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Line {
    pub txn: String,
    pub state: State,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TxnKind>,
    /// The file the rename lands on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// The file read: the original, or the backup for an undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_temp: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
    /// The backup's final name, journaled before the backup is renamed to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_target: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_mtime: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undoes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_blake3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_blake3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl Line {
    /// A line for `txn` entering `state` now, with no other field.
    pub fn new(txn: &str, state: State) -> Self {
        Self {
            txn: txn.to_owned(),
            state,
            at: super::fsx::utc_timestamp(std::time::SystemTime::now()),
            kind: None,
            path: None,
            source: None,
            temp: None,
            backup_temp: None,
            backup: None,
            backup_target: None,
            keep_mtime: None,
            sidecar: None,
            undoes: None,
            original_blake3: None,
            original_bytes: None,
            output_blake3: None,
            output_bytes: None,
            record: None,
            outcome: None,
            error: None,
            notes: Vec::new(),
        }
    }
}

/// A transaction as its journal lines describe it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Transaction id.
    pub txn: String,
    /// What it does.
    pub kind: TxnKind,
    /// Its last state.
    pub state: State,
    /// The furthest step it reached before ending (equal to `state` while it runs).
    pub reached: State,
    /// When it was planned (RFC 3339, UTC).
    pub started_at: String,
    /// The file the rename lands on.
    pub path: PathBuf,
    /// The file read: the original, or the backup for an undo.
    pub source: PathBuf,
    /// The temp file next to `path`.
    pub temp: PathBuf,
    /// Where the backup is first written (in place only).
    pub backup_temp: Option<PathBuf>,
    /// The backup, once made (in place), or the backup restored (undo).
    pub backup: Option<PathBuf>,
    /// The name the backup was being renamed to (it may exist without `backed_up`).
    pub backup_target: Option<PathBuf>,
    /// Whether the modification time is restored.
    pub keep_mtime: bool,
    /// Whether a sidecar is written.
    pub sidecar: bool,
    /// For an undo: the transaction it undoes.
    pub undoes: Option<String>,
    /// BLAKE3 (hex) of the original file.
    pub original_blake3: Option<String>,
    /// Length of the original file, bytes.
    pub original_bytes: Option<u64>,
    /// BLAKE3 (hex) of the file the rename puts in place.
    pub output_blake3: Option<String>,
    /// Length of that file, bytes.
    pub output_bytes: Option<u64>,
    /// The request and what the render did.
    pub record: Option<Record>,
    /// How recovery ended it.
    pub outcome: Option<Outcome>,
    /// Why it failed.
    pub error: Option<String>,
    /// Notes (extended attributes not restored, ...).
    pub notes: Vec<String>,
    /// Whether a later undo put the original back.
    pub undone: bool,
}

impl Entry {
    /// Whether the change reached its target: done, or completed by recovery.
    #[must_use]
    pub fn completed(&self) -> bool {
        self.state == State::Done
            || (self.state == State::Recovered && self.outcome == Some(Outcome::Completed))
    }

    fn from_planned(line: Line) -> Option<Self> {
        Some(Self {
            kind: line.kind?,
            path: line.path?,
            source: line.source?,
            temp: line.temp?,
            txn: line.txn,
            state: line.state,
            reached: line.state,
            started_at: line.at,
            backup_temp: line.backup_temp,
            backup: line.backup,
            backup_target: line.backup_target,
            keep_mtime: line.keep_mtime.unwrap_or(true),
            sidecar: line.sidecar.unwrap_or(true),
            undoes: line.undoes,
            original_blake3: line.original_blake3,
            original_bytes: line.original_bytes,
            output_blake3: None,
            output_bytes: None,
            record: None,
            outcome: None,
            error: None,
            notes: Vec::new(),
            undone: false,
        })
    }

    fn apply(&mut self, line: Line) {
        self.state = line.state;
        if !line.state.is_final() {
            self.reached = self.reached.max(line.state);
        }
        self.backup = line.backup.or(self.backup.take());
        self.backup_target = line.backup_target.or(self.backup_target.take());
        self.original_blake3 = line.original_blake3.or(self.original_blake3.take());
        self.original_bytes = line.original_bytes.or(self.original_bytes);
        self.output_blake3 = line.output_blake3.or(self.output_blake3.take());
        self.output_bytes = line.output_bytes.or(self.output_bytes);
        self.record = line.record.or(self.record.take());
        self.outcome = line.outcome.or(self.outcome);
        self.error = line.error.or(self.error.take());
        self.notes.extend(line.notes);
    }
}

/// One [`Entry`] per transaction, in the order they were planned; lines of a transaction whose
/// `planned` line is missing are skipped. An undo that completed marks what it undid.
pub(crate) fn fold(lines: Vec<Line>) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for line in lines {
        if let Some(&i) = index.get(&line.txn) {
            entries[i].apply(line);
        } else if line.state == State::Planned {
            let txn = line.txn.clone();
            if let Some(e) = Entry::from_planned(line) {
                index.insert(txn, entries.len());
                entries.push(e);
            }
        }
    }
    let undone: Vec<String> = entries
        .iter()
        .filter(|e| e.kind == TxnKind::Undo && e.completed())
        .filter_map(|e| e.undoes.clone())
        .collect();
    for e in &mut entries {
        e.undone = undone.contains(&e.txn);
    }
    entries
}

/// The journal of one backup root.
#[derive(Debug, Clone)]
pub(crate) struct Journal {
    root: PathBuf,
    path: PathBuf,
}

impl Journal {
    /// The journal in `root`, which is created if needed.
    ///
    /// # Errors
    /// [`Error::Io`] naming `root`.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join(LOCK_DIR)).map_err(|e| io_err(root, e))?;
        Ok(Self {
            root: root.to_path_buf(),
            path: root.join(JOURNAL_FILE),
        })
    }

    /// The backup root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Appends `line` and syncs the journal, under an exclusive lock so lines of concurrent
    /// transactions never interleave.
    ///
    /// # Errors
    /// [`Error::Io`] naming the journal.
    pub fn append(&self, line: &Line) -> Result<()> {
        let mut text = serde_json::to_string(line)
            .map_err(|e| Error::Internal(format!("journal line not serialisable: {e}")))?;
        text.push('\n');
        let path = &self.path;
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| io_err(path, e))?;
        file.lock().map_err(|e| io_err(path, e))?;
        if !ends_with_newline(&mut file).map_err(|e| io_err(path, e))? {
            // A crash cut the last line short: end it, so this record stands on its own line
            // (the cut one is skipped when read).
            text.insert(0, '\n');
        }
        file.write_all(text.as_bytes())
            .map_err(|e| io_err(path, e))?;
        sync_file(&file, path)?;
        tracing::debug!(txn = %line.txn, stage = line.state.as_str(), "journaled");
        Ok(())
    }

    /// Every transaction in the journal (none when it does not exist yet).
    ///
    /// # Errors
    /// [`Error::Io`] naming the journal.
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let path = &self.path;
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(path, e)),
        };
        file.lock_shared().map_err(|e| io_err(path, e))?;
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|e| io_err(path, e))?;
        drop(file);
        let mut lines = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Line>(raw) {
                Ok(line) => lines.push(line),
                Err(e) => {
                    tracing::warn!(path = %path.display(), line = n + 1, error = %e, "journal line skipped");
                }
            }
        }
        Ok(fold(lines))
    }

    /// The transaction `txn`, if journaled.
    ///
    /// # Errors
    /// As for [`Self::entries`].
    pub fn entry(&self, txn: &str) -> Result<Option<Entry>> {
        Ok(self.entries()?.into_iter().find(|e| e.txn == txn))
    }

    fn lock_path(&self, txn: &str) -> PathBuf {
        self.root.join(LOCK_DIR).join(format!("{txn}.lock"))
    }
}

/// The exclusive lock on one target path, held for a whole transaction so two transactions
/// on the same file run one after the other. The lock file (`locks/target-<BLAKE3 of the
/// path>.lock`) is removed when the lock is dropped; a waiter that wakes up on a removed file
/// takes the lock again on the new one.
#[derive(Debug)]
pub(crate) struct TargetLock {
    _held: TxnLock,
}

impl TargetLock {
    /// Waits for and takes the lock on `target`.
    ///
    /// # Errors
    /// [`Error::Io`] naming the lock file.
    pub fn acquire(journal: &Journal, target: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let key = blake3::hash(target.as_os_str().as_encoded_bytes());
        let path = journal
            .root
            .join(LOCK_DIR)
            .join(format!("target-{}.lock", &key.to_hex()[..32]));
        loop {
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| io_err(&path, e))?;
            file.lock().map_err(|e| io_err(&path, e))?;
            // Still the file at `path`, not one its last holder removed meanwhile?
            let held = file.metadata().map_err(|e| io_err(&path, e))?;
            let linked = std::fs::metadata(&path)
                .is_ok_and(|m| m.dev() == held.dev() && m.ino() == held.ino());
            if linked {
                return Ok(Self {
                    _held: TxnLock {
                        file: Some(file),
                        path,
                    },
                });
            }
        }
    }
}

impl Journal {
    /// Removes target lock files nobody holds (left by a crash). A waiter that opened one
    /// before it went notices it was removed and takes a new one (see [`TargetLock`]).
    pub fn sweep_target_locks(&self) {
        let dir = self.root.join(LOCK_DIR);
        let Ok(list) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in list.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("target-") {
                continue;
            }
            let path = entry.path();
            let Ok(file) = OpenOptions::new().write(true).open(&path) else {
                continue;
            };
            if file.try_lock().is_ok() {
                let _ = remove_if_exists(&path);
            }
        }
    }
}

/// Whether `file` is empty or its last byte is a newline.
fn ends_with_newline(file: &mut File) -> std::io::Result<bool> {
    use std::io::{Seek, SeekFrom};
    if file.seek(SeekFrom::End(0))? == 0 {
        return Ok(true);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0_u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] == b'\n')
}

/// The exclusive lock a running transaction holds on its lock file; dropping it removes the
/// file and releases the lock.
#[derive(Debug)]
pub(crate) struct TxnLock {
    file: Option<File>,
    path: PathBuf,
}

impl TxnLock {
    /// Takes the lock of `txn`, creating its lock file; `None` when another process or thread
    /// holds it (the transaction is running).
    ///
    /// # Errors
    /// [`Error::Io`] naming the lock file.
    pub fn try_acquire(journal: &Journal, txn: &str) -> Result<Option<Self>> {
        let path = journal.lock_path(txn);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| io_err(&path, e))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                file: Some(file),
                path,
            })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(io_err(&path, e)),
        }
    }
}

impl Drop for TxnLock {
    fn drop(&mut self) {
        if let Err(e) = remove_if_exists(&self.path) {
            tracing::warn!(path = %self.path.display(), error = %e, "lock file not removed");
        }
        drop(self.file.take());
    }
}

#[cfg(test)]
mod tests;
