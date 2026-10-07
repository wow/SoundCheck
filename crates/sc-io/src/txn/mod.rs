//! Write transactions: replacing a file by its rendered version so that, whatever happens, the
//! path holds either the original or a verified output, never neither and never a partial
//! file; with a backup, a journal, crash recovery and undo.
//!
//! [`Transaction::apply_in_place`] runs these steps; each state change is one synced line in
//! `journal.jsonl` in the backup root (see [`journal`]):
//!
//! 1. **Preflight** (nothing created yet, not even the backup root or a destination folder):
//!    the path is resolved to the name the file system holds (so `Track.wav` and `track.wav`,
//!    or precomposed and decomposed accents, are one file in the journal). Refused: a path in
//!    the backup root ([`sc_core::Error::InvalidArgument`]; library scanners must skip it, see
//!    [`is_under_backup_root`]); a file on a rekordbox USB export (a `PIONEER` folder at its
//!    volume's root, [`sc_core::Error::RekordboxUsbExport`]); a symbolic link, a
//!    Finder-locked file, a file of another user, a file with other hard links, a file with an
//!    access control list (detected through `exacl` on macOS and the POSIX ACL attribute on
//!    Linux: a renamed file would silently lose it), a read-only file or folder
//!    ([`sc_core::Error::InPlaceRefused`]); a container other than WAV, RF64, AIFF, AIFF-C or
//!    FLAC ([`sc_core::Error::UnsupportedFormat`]); and a volume that would keep less than
//!    64 MiB free: the file's volume must hold an upper bound of the output, the backup's
//!    volume a copy of the original ([`sc_core::Error::NoSpace`]). The file's identity is read.
//!    Then the target is locked for the whole transaction (a second transaction on it waits,
//!    polling every 100 ms and honouring the cancel flag), the in-place checks run again, a
//!    file changed meanwhile (another transaction ran first) is refused
//!    ([`sc_core::Error::FileChanged`]: it is never rendered twice), and the original's metadata (extended attributes at full size, dates, mode,
//!    owner, group) and identity (device, inode, length, modification and change time) are
//!    read. The `planned` line also records the target volume's lasting identity (see
//!    [`volume::volume_identity`]) and the target folder's inode and birth time. State
//!    `planned`.
//! 2. **Temp file**: `.<name>.soundcheck-tmp-<id>` in the same folder, created new by the
//!    render ([`crate::render::apply_iff`], or the FLAC render with its verification deferred),
//!    then synced (`F_FULLFSYNC`; plain `fsync` only where a volume does not support it).
//!    `temp_written`.
//! 3. **Verify**: the synced file is read back (WAV/AIFF: header, audio hash, and every carried
//!    chunk and the trailing bytes against the source; FLAC: the full independent decode) and
//!    hashed whole (BLAKE3). `verified`.
//! 4. **Backup**: with the original's identity unchanged, it is copied by the system (an APFS
//!    clone or `copyfile(3)`: data, extended attributes, the whole resource fork) to
//!    `<backup root>/<yyyy-mm-dd>/<volume name>/<path relative to the volume>` (the local date;
//!    the volume rule is in [`volume`]), first under a temp name, synced, hashed by reading it
//!    back, given the original's dates and mode; its final name (the first free one of
//!    `a.wav`, `a (2).wav`, ...: a backup is never overwritten) is journaled before the rename
//!    to it. Its BLAKE3 is the original's. `backed_up`.
//! 5. **Rename**: if the original's identity is still unchanged, the temp file is renamed over
//!    it (atomic within a folder) and the folder synced (errors ignored). `renamed`. This is
//!    the only step that changes the path, and nothing before it does.
//! 6. **Metadata**: every extended attribute (read back to check), the group, the creation
//!    date, the modification time (with [`TxnOptions::keep_mtime`]) and the mode of the
//!    original. `metadata_done`.
//! 7. **Sidecar** `<file>.soundcheck.json` next to the file (see [`sidecar`]), written
//!    atomically. `done`.
//!
//! A failure or a cancel before the rename removes the temp file (and the backup, once the file
//! is checked to hold the original) and journals `failed`; the original is untouched. When
//! cleanup fails, or a failed rename may have happened, nothing is guessed: the transaction
//! stays pending for [`recover`]. After the rename nothing is undone: what could not be
//! restored is reported in [`TxnReport::notes`]. [`Transaction::apply_to_folder`] does the
//! same into another folder without a backup and never replaces an existing file (on exFAT and
//! FAT volumes, which have no exclusive rename, through an empty placeholder created
//! exclusively). [`recover`] finishes or rolls back transactions a crash interrupted and leaves
//! unreachable ones (an unmounted volume) pending (call it at start and before every batch);
//! [`forget`] ends a pending one the user gives up on (state `forgotten`, every file it left
//! kept), so recovery skips it from then on. [`Transaction::undo`] puts the newest backup of a
//! file back through the same steps. With
//! the `crash-test` feature (tests only) the process aborts at the crash point named by
//! `SC_TEST_CRASH_AFTER_STEP` (see [`CRASH_ENV`]).

mod apply;
pub(crate) mod crash;
mod finish;
mod forget;
mod fsx;
pub mod journal;
pub(crate) mod meta;
mod preflight;
mod recover;
pub mod sidecar;
mod undo;
mod verify;
pub mod volume;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use sc_core::{Error, RenderRequest, Result};

use crate::render::RenderReport;

pub use crash::CRASH_ENV;
pub use forget::{Forgotten, forget};
pub use fsx::{TEMP_MARKER, hex};
pub use journal::{Entry, JOURNAL_FILE, Outcome, State, TxnKind};
pub use preflight::{SPACE_MARGIN_BYTES, is_under_backup_root};
pub use recover::{Pending, Recovered, RecoveryReport, recover};
pub use sidecar::{SIDECAR_SUFFIX, sidecar_path};
pub use undo::{SidecarAfterUndo, UndoReport};
pub use volume::{SystemVolumes, Volume, VolumeProvider};

/// Environment variable that overrides the default backup root (used by tests).
pub const BACKUP_ROOT_ENV: &str = "SC_BACKUP_ROOT";

/// Name of the default backup folder in the user's Music folder.
pub const BACKUP_FOLDER_NAME: &str = "SoundCheck Backups";

/// The default backup root: `$SC_BACKUP_ROOT` when set, else `~/Music/SoundCheck Backups`.
///
/// # Errors
/// [`Error::Internal`] when the system names no home folder.
pub fn default_backup_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os(BACKUP_ROOT_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    let dirs = directories::UserDirs::new()
        .ok_or_else(|| Error::Internal("no home folder to keep backups in".into()))?;
    let music = dirs
        .audio_dir()
        .map_or_else(|| dirs.home_dir().join("Music"), Path::to_path_buf);
    Ok(music.join(BACKUP_FOLDER_NAME))
}

/// How a transaction runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxnOptions {
    /// Where backups and the journal live.
    pub backup_root: PathBuf,
    /// Restore the original's modification time (DJ apps then do not see a changed file).
    pub keep_mtime: bool,
    /// Write `<file>.soundcheck.json` next to the file.
    pub sidecar: bool,
}

impl TxnOptions {
    /// Backups in `backup_root`, modification time kept, sidecar written.
    #[must_use]
    pub fn new(backup_root: impl Into<PathBuf>) -> Self {
        Self {
            backup_root: backup_root.into(),
            keep_mtime: true,
            sidecar: true,
        }
    }
}

/// What a transaction did.
#[derive(Debug, Clone, PartialEq)]
pub struct TxnReport {
    /// Transaction id (as in the journal).
    pub txn: String,
    /// In place or to a folder.
    pub kind: TxnKind,
    /// The file read.
    pub source: PathBuf,
    /// The file written (the source's path in place).
    pub output: PathBuf,
    /// The backup of the original (in place).
    pub backup: Option<PathBuf>,
    /// The sidecar written.
    pub sidecar: Option<PathBuf>,
    /// What the render did.
    pub render: RenderReport,
    /// BLAKE3 of the original file.
    pub original_blake3: [u8; 32],
    /// BLAKE3 of the output file.
    pub output_blake3: [u8; 32],
    /// Length of the output file, bytes.
    pub output_bytes: u64,
    /// What could not be restored or written after the rename (extended attributes the system
    /// refused, a sidecar that could not be written).
    pub notes: Vec<String>,
    /// Time spent up to each journaled state.
    pub timings: Vec<(State, Duration)>,
}

/// Runs transactions against a source of volume facts ([`SystemVolumes`] unless a test
/// simulates volumes).
pub struct Transaction<'v> {
    volumes: &'v dyn VolumeProvider,
    /// Fault injection for unit tests.
    #[cfg(test)]
    pub(crate) hooks: Hooks,
}

/// Functions unit tests run inside a transaction to inject faults.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Hooks {
    /// Runs after the checks, before waiting for the target lock.
    pub after_check: Option<fn()>,
    /// Runs with the temp file and the source right after the temp file is synced.
    pub after_temp_written: Option<fn(&Path, &Path)>,
    /// Runs with the temp file and the target right before the rename.
    pub before_rename: Option<fn(&Path, &Path)>,
}

impl std::fmt::Debug for Transaction<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction").finish_non_exhaustive()
    }
}

static SYSTEM_VOLUMES: SystemVolumes = SystemVolumes;

impl Default for Transaction<'static> {
    fn default() -> Self {
        Self::with_volumes(&SYSTEM_VOLUMES)
    }
}

impl<'v> Transaction<'v> {
    /// Transactions that ask `volumes` about volumes.
    #[must_use]
    pub fn with_volumes(volumes: &'v dyn VolumeProvider) -> Self {
        Self {
            volumes,
            #[cfg(test)]
            hooks: Hooks::default(),
        }
    }

    /// Replaces the file at `path` by its render with `req`, after backing it up (the steps in
    /// the module documentation); `cancel` is honoured until the rename.
    ///
    /// # Errors
    /// The preflight refusals, the render's errors, [`Error::VerifyFailed`],
    /// [`Error::FileChanged`] when the file changed during processing, [`Error::Cancelled`],
    /// [`Error::Io`]. On any error before the rename the file is untouched and nothing is left
    /// behind; an error after it (journal not writable) leaves the transaction for
    /// [`recover`].
    pub fn apply_in_place(
        &self,
        path: &Path,
        req: &RenderRequest,
        opts: &TxnOptions,
        cancel: &AtomicBool,
    ) -> Result<TxnReport> {
        apply::apply(self, path, None, req, opts, cancel)
    }

    /// Writes the render of `path` with `req` into `out_dir` (created if needed) under the same
    /// name, with the source's metadata, verified, without a backup; the source is untouched
    /// and an existing file is never replaced ([`Error::AlreadyExists`]).
    ///
    /// # Errors
    /// As for [`Self::apply_in_place`], the in-place refusals excepted;
    /// [`Error::RekordboxUsbExport`] for a destination on a rekordbox USB export.
    pub fn apply_to_folder(
        &self,
        path: &Path,
        out_dir: &Path,
        req: &RenderRequest,
        opts: &TxnOptions,
        cancel: &AtomicBool,
    ) -> Result<TxnReport> {
        apply::apply(self, path, Some(out_dir), req, opts, cancel)
    }

    /// Puts back the original of the newest change of `path` recorded in `backup_root`.
    ///
    /// # Errors
    /// [`Error::NothingToUndo`]; [`Error::FileChanged`] when the file is no longer what
    /// SoundCheck wrote; [`Error::VerifyFailed`] when the backup no longer matches the
    /// original's hash; the in-place refusals; [`Error::Io`].
    pub fn undo(&self, path: &Path, backup_root: &Path) -> Result<UndoReport> {
        undo::undo(self, path, backup_root)
    }
}

/// [`Transaction::apply_in_place`] with the system's volumes.
///
/// # Errors
/// As for [`Transaction::apply_in_place`].
pub fn apply_in_place(
    path: &Path,
    req: &RenderRequest,
    opts: &TxnOptions,
    cancel: &AtomicBool,
) -> Result<TxnReport> {
    Transaction::default().apply_in_place(path, req, opts, cancel)
}

/// [`Transaction::apply_to_folder`] with the system's volumes.
///
/// # Errors
/// As for [`Transaction::apply_to_folder`].
pub fn apply_to_folder(
    path: &Path,
    out_dir: &Path,
    req: &RenderRequest,
    opts: &TxnOptions,
    cancel: &AtomicBool,
) -> Result<TxnReport> {
    Transaction::default().apply_to_folder(path, out_dir, req, opts, cancel)
}

/// [`Transaction::undo`] with the system's volumes.
///
/// # Errors
/// As for [`Transaction::undo`].
pub fn undo(path: &Path, backup_root: &Path) -> Result<UndoReport> {
    Transaction::default().undo(path, backup_root)
}

/// Every transaction journaled in `backup_root`, oldest first (none without a journal).
///
/// # Errors
/// [`Error::Io`] when the journal cannot be read.
pub fn journal_entries(backup_root: &Path) -> Result<Vec<Entry>> {
    if !backup_root.is_dir() {
        return Ok(Vec::new());
    }
    journal::Journal::open(backup_root)?.entries()
}

#[cfg(test)]
pub(crate) mod tests;
