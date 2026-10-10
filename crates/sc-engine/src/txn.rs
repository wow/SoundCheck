//! Writing a processed file, through the write transaction of `sc_io::txn` (temp file, verify,
//! backup, atomic rename, metadata, sidecar, journal), and the crash recovery that must run
//! before anything is written.
//!
//! [`apply_file`] is the one entry point the CLI and the app's export use: in place (the
//! original backed up first) or as a copy into a folder; [`undo_file`] puts an original back. [`recover_at_start`] finishes or rolls
//! back the transactions a crash interrupted; the app shell calls it once at start and the CLI
//! before every command that writes.

mod inputs;
mod tags;

use std::path::{Path, PathBuf};

use sc_core::ipc::{PendingChange, RecoveredChange, RecoveryOutcome, RecoveryStatus};
use sc_core::{BextLoudness, RenderRequest, Result};
use sc_io::txn::{self, Outcome, RecoveryReport, TxnOptions, TxnReport, UndoReport};

use crate::CancelToken;

pub use inputs::check_inputs;
pub use tags::{Tag, check_tags, tag_edits};

/// What [`apply_file`] does to one file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ApplyRequest {
    /// Gain applied to every sample, dB (finite; 0 keeps the samples bit for bit).
    pub gain_db: f64,
    /// Frames to cut from the start; the render makes the cut up to 1 ms earlier at the
    /// quietest frame and fades it in over 2 ms (the cut made is in the report's render),
    /// unless [`Self::trim_snapped_from_frames`] says it already was.
    pub trim_frames: u64,
    /// Set when `trim_frames` is already the snapped cut (`sc_io::render::snap_head_cut`) of
    /// this requested cut: the render cuts exactly `trim_frames` (see
    /// [`RenderRequest::trim_snapped_from_frames`]).
    pub trim_snapped_from_frames: Option<u64>,
    /// Output bits per sample, 16 or 24; `None` keeps the source depth.
    pub bits: Option<u8>,
    /// Loudness written into an existing `bext` chunk.
    pub loudness: Option<BextLoudness>,
    /// Tag items by their neutral names (see [`tags`]), mapped to the file's container.
    pub tags: Vec<Tag>,
}

/// Where the processed file goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// Replaces the original, after backing it up into the backup root.
    InPlace,
    /// A copy with the same name in this folder (created if needed); the original is untouched
    /// and an existing file is never replaced.
    Folder(PathBuf),
}

/// How [`apply_file`] writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOptions {
    /// In place or into a folder.
    pub place: Place,
    /// Where backups and the journal live (`sc_io::txn::default_backup_root` unless the user
    /// chose another).
    pub backup_root: PathBuf,
    /// Restore the original's modification time, so DJ apps do not see a changed file.
    pub keep_mtime: bool,
    /// Write `<file>.soundcheck.json` next to the output.
    pub sidecar: bool,
}

impl ApplyOptions {
    /// In place, backups in `backup_root`, modification time kept, sidecar written.
    #[must_use]
    pub fn in_place(backup_root: impl Into<PathBuf>) -> Self {
        Self {
            place: Place::InPlace,
            backup_root: backup_root.into(),
            keep_mtime: true,
            sidecar: true,
        }
    }
}

/// Renders `path` with `req` and writes it as `opts` says, verified, through one journaled
/// transaction; `cancel` is honoured until the output replaces anything. A request over many
/// files checks them together first with [`check_inputs`].
///
/// # Errors
/// `InvalidArgument` for a tag [`check_tags`] refuses; the transaction's refusals and failures (see `sc_io::txn`): `RekordboxUsbExport`,
/// `InPlaceRefused`, `NoSpace`, `UnsupportedFormat`, `NotDjSafe`, `WouldClip`,
/// `VerifyFailed`, `FileChanged`, `AlreadyExists`, `Cancelled`, `Io`. On any error before the
/// rename the original is untouched and nothing is left behind.
pub fn apply_file(
    path: &Path,
    req: &ApplyRequest,
    opts: &ApplyOptions,
    cancel: &CancelToken,
) -> Result<TxnReport> {
    let tag_edits = if req.tags.is_empty() {
        Vec::new()
    } else {
        check_tags(&req.tags)?;
        tag_edits(&req.tags, txn::tag_family(path)?)
    };
    let render = RenderRequest {
        gain_db: req.gain_db,
        trim_frames: req.trim_frames,
        trim_snapped_from_frames: req.trim_snapped_from_frames,
        bits: req.bits,
        loudness: req.loudness,
        tag_edits,
    };
    let txn_opts = TxnOptions {
        backup_root: opts.backup_root.clone(),
        keep_mtime: opts.keep_mtime,
        sidecar: opts.sidecar,
    };
    let flag = cancel.flag();
    let report = match &opts.place {
        Place::InPlace => txn::apply_in_place(path, &render, &txn_opts, &flag),
        Place::Folder(dir) => txn::apply_to_folder(path, dir, &render, &txn_opts, &flag),
    };
    match &report {
        Ok(r) => tracing::info!(
            path = %path.display(),
            txn = %r.txn,
            gain_db = req.gain_db,
            trim_frames = req.trim_frames,
            "applied"
        ),
        Err(e) => tracing::info!(path = %path.display(), error = %e, "not applied"),
    }
    report
}

/// Puts the original of the newest change of `path` recorded in `backup_root` back in place,
/// through the same transaction steps (see `sc_io::txn::undo`).
///
/// # Errors
/// `NothingToUndo`, `FileChanged` when the file is no longer what SoundCheck wrote,
/// `VerifyFailed` when the backup no longer matches, the in-place refusals, `Io`.
pub fn undo_file(path: &Path, backup_root: &Path) -> Result<UndoReport> {
    let report = txn::undo(path, backup_root);
    match &report {
        Ok(r) => tracing::info!(path = %path.display(), txn = %r.txn, undone = %r.undone, "undone"),
        Err(e) => tracing::info!(path = %path.display(), error = %e, "not undone"),
    }
    report
}

/// Recovers the transactions a crash interrupted in `backup_root` (see `sc_io::txn::recover`)
/// and logs what it did. Never fails: a journal that cannot be read is reported as
/// [`RecoveryStatus::Failed`], so the caller can still start.
#[must_use]
pub fn recover_at_start(backup_root: &Path) -> RecoveryStatus {
    match txn::recover(backup_root) {
        Ok(report) => {
            if report.recovered.is_empty() && report.pending.is_empty() {
                tracing::debug!(root = %backup_root.display(), "nothing to recover");
            } else {
                tracing::info!(
                    root = %backup_root.display(),
                    recovered = report.recovered.len(),
                    pending = report.pending.len(),
                    "recovery ran"
                );
            }
            for p in &report.pending {
                tracing::warn!(txn = %p.txn, path = %p.path.display(), reason = %p.reason, "change left pending");
            }
            recovery_status(&report)
        }
        Err(e) => {
            tracing::warn!(root = %backup_root.display(), error = %e, "recovery could not run");
            RecoveryStatus::Failed {
                message: e.to_string(),
            }
        }
    }
}

/// `report` as the app shows it.
#[must_use]
pub fn recovery_status(report: &RecoveryReport) -> RecoveryStatus {
    RecoveryStatus::Finished {
        recovered: report
            .recovered
            .iter()
            .map(|r| RecoveredChange {
                txn: r.txn.clone(),
                path: r.path.display().to_string(),
                outcome: match r.outcome {
                    Outcome::Completed => RecoveryOutcome::Completed,
                    Outcome::RolledBack => RecoveryOutcome::RolledBack,
                },
            })
            .collect(),
        pending: report
            .pending
            .iter()
            .map(|p| PendingChange {
                txn: p.txn.clone(),
                path: p.path.display().to_string(),
                reason: p.reason.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
