//! Finishing or rolling back transactions a crash interrupted.
//!
//! For every journaled transaction that has not ended and whose lock can be taken (so no live
//! process runs it), recovery decides from the furthest state it reached:
//!
//! - **Before the rename** (in place: up to `backed_up`; to a folder and undo: up to
//!   `verified`): the temp file and the backup's temp file are deleted, and a backup made for
//!   it is deleted once the target is checked to still hold the original (same BLAKE3); the
//!   target was never touched. Outcome [`Outcome::RolledBack`].
//! - **At the last state before the rename with the temp file gone**: the rename may have
//!   happened just before the crash; when the target's BLAKE3 equals the verified output's,
//!   it is treated as renamed, else as rolled back.
//! - **Renamed or later**: the metadata is restored again (from the backup for in place, which
//!   holds the original's; from the source for a copy or an undo) and the sidecar written (or,
//!   for an undo, settled). Outcome [`Outcome::Completed`].
//!
//! Each recovered transaction gets a `recovered` line, so running recovery again finds nothing
//! to do.

use std::path::{Path, PathBuf};

use sc_core::{Error, Result};

use super::apply::metadata_step;
use super::fsx::{exists, hash_file, hex, remove_if_exists};
use super::journal::{Entry, Journal, Line, Outcome, State, TxnKind, TxnLock};
use super::meta::snapshot;
use super::{sidecar, undo};

/// A transaction recovery ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// Transaction id.
    pub txn: String,
    /// What it did.
    pub kind: TxnKind,
    /// The file it targeted.
    pub path: PathBuf,
    /// The furthest state it had reached.
    pub reached: State,
    /// How recovery ended it.
    pub outcome: Outcome,
    /// What could not be restored or removed.
    pub notes: Vec<String>,
}

/// What recovery does with an interrupted transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Delete what it created; the target is untouched.
    RollBack,
    /// Finish metadata and sidecar; the target holds the output.
    Complete,
    /// Compare the target with the output to learn whether the rename happened.
    CheckTarget,
}

/// The decision table: `reached` is the furthest state, `temp_exists` whether the temp file is
/// still there (a rename moves it away).
pub(crate) fn decide(kind: TxnKind, reached: State, temp_exists: bool) -> Action {
    if reached >= State::Renamed {
        Action::Complete
    } else if reached == kind.pre_rename() && !temp_exists {
        Action::CheckTarget
    } else {
        Action::RollBack
    }
}

/// Recovers every interrupted transaction in the journal of `backup_root` (nothing when there
/// is no journal).
///
/// # Errors
/// [`Error::Io`] when the journal or a file cannot be read, written or removed; the
/// transactions recovered before the error keep their `recovered` lines.
pub fn recover(backup_root: &Path) -> Result<Vec<Recovered>> {
    if !backup_root.is_dir() {
        return Ok(Vec::new());
    }
    let journal = Journal::open(backup_root)?;
    let pending: Vec<String> = journal
        .entries()?
        .into_iter()
        .filter(|e| !e.state.is_final())
        .map(|e| e.txn)
        .collect();
    let mut out = Vec::new();
    for txn in pending {
        let Some(lock) = TxnLock::try_acquire(&journal, &txn)? else {
            tracing::debug!(txn, stage = "recover", "running; skipped");
            continue;
        };
        // Read again under the lock: it may have ended meanwhile.
        let Some(entry) = journal.entry(&txn)? else {
            continue;
        };
        if entry.state.is_final() {
            continue;
        }
        let recovered = recover_one(&journal, &entry)?;
        tracing::info!(
            txn,
            path = %entry.path.display(),
            reached = entry.reached.as_str(),
            outcome = ?recovered.outcome,
            "recovered"
        );
        out.push(recovered);
        drop(lock);
    }
    Ok(out)
}

/// Whether the file at `path` hashes to `want` (hex).
fn holds(path: &Path, want: Option<&str>) -> bool {
    match (hash_file(path), want) {
        (Ok((_, h)), Some(w)) => hex(&h) == w,
        _ => false,
    }
}

fn recover_one(journal: &Journal, e: &Entry) -> Result<Recovered> {
    let action = match decide(e.kind, e.reached, exists(&e.temp)) {
        Action::CheckTarget if holds(&e.path, e.output_blake3.as_deref()) => Action::Complete,
        Action::CheckTarget => Action::RollBack,
        other => other,
    };
    let (outcome, notes) = match action {
        Action::Complete => (Outcome::Completed, complete(journal, e)?),
        Action::RollBack | Action::CheckTarget => (Outcome::RolledBack, roll_back(e)?),
    };
    let mut line = Line::new(&e.txn, State::Recovered);
    line.outcome = Some(outcome);
    line.notes.clone_from(&notes);
    journal.append(&line)?;
    Ok(Recovered {
        txn: e.txn.clone(),
        kind: e.kind,
        path: e.path.clone(),
        reached: e.reached,
        outcome,
        notes,
    })
}

fn roll_back(e: &Entry) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    remove_if_exists(&e.temp)?;
    if let Some(t) = &e.backup_temp {
        remove_if_exists(t)?;
    }
    if e.kind == TxnKind::InPlace
        && let Some(backup) = &e.backup
        && exists(backup)
    {
        if holds(&e.path, e.original_blake3.as_deref()) {
            remove_if_exists(backup)?;
        } else {
            notes.push(format!(
                "backup kept at {}: the file no longer matches it",
                backup.display()
            ));
        }
    }
    Ok(notes)
}

fn complete(journal: &Journal, e: &Entry) -> Result<Vec<String>> {
    let meta_source = match e.kind {
        TxnKind::InPlace => e.backup.as_deref().ok_or_else(|| {
            Error::Internal(format!(
                "transaction {} was renamed without a backup",
                e.txn
            ))
        })?,
        TxnKind::ToFolder | TxnKind::Undo => e.source.as_path(),
    };
    let mut notes = match snapshot(meta_source) {
        Ok(meta) => metadata_step(&e.path, &meta, e.kind == TxnKind::Undo || e.keep_mtime),
        Err(err) => vec![format!("metadata not restored: {err}")],
    };
    match e.kind {
        TxnKind::Undo => {
            let entries = journal.entries()?;
            if let Some(undone) = &e.undoes {
                undo::settle_sidecar(&entries, &e.path, undone)?;
            }
        }
        _ if e.sidecar => {
            if let Err(err) = sidecar::write(e, &notes) {
                notes.push(format!("sidecar not written: {err}"));
            }
        }
        _ => {}
    }
    Ok(notes)
}

#[cfg(test)]
mod tests;
