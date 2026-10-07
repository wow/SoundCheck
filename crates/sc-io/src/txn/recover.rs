//! Finishing or rolling back transactions a crash or a failure interrupted.
//!
//! For every journaled transaction that has not ended and whose lock can be taken (so no live
//! process runs it), recovery decides from the furthest state it reached:
//!
//! - **Before the rename** (in place: up to `backed_up`; to a folder and undo: up to
//!   `verified`): the temp file and the backup's temp file are deleted, an empty placeholder
//!   left at a no-replace target is deleted, and a backup made for it is deleted once the
//!   target is checked to still hold the original (same BLAKE3; otherwise it is kept and a
//!   note says so); the target was never touched. Outcome [`Outcome::RolledBack`].
//! - **At the last state before the rename with the temp file gone**: the rename may have
//!   happened just before the crash; when the target's BLAKE3 equals the verified output's,
//!   it is treated as renamed, else as rolled back.
//! - **Renamed or later**: the metadata is restored again (from the backup for in place, which
//!   holds the original's; from the source for a copy or an undo) and the sidecar written (or,
//!   for an undo, settled). Outcome [`Outcome::Completed`].
//!
//! A transaction whose target folder cannot be reached (an unmounted volume), whose cleanup
//! fails, or whose lock is held stays pending, untouched, and is listed in
//! [`RecoveryReport::pending`]; a later recovery tries again. One transaction's failure never
//! stops the others. Each recovered transaction gets a `recovered` line, so running recovery
//! again finds nothing to do.

use std::path::{Path, PathBuf};

use sc_core::{Error, Result};

use super::finish::metadata_step;
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

/// A transaction recovery left for later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// Transaction id.
    pub txn: String,
    /// The file it targets.
    pub path: PathBuf,
    /// Why it was left (running, folder not reachable, cleanup failed).
    pub reason: String,
}

/// What [`recover`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Transactions finished or rolled back now.
    pub recovered: Vec<Recovered>,
    /// Transactions left pending.
    pub pending: Vec<Pending>,
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
/// [`Error::Io`] when the journal cannot be read; a failure on one transaction makes it
/// pending instead.
pub fn recover(backup_root: &Path) -> Result<RecoveryReport> {
    let mut report = RecoveryReport::default();
    if !backup_root.is_dir() {
        return Ok(report);
    }
    let journal = Journal::open(backup_root)?;
    let pending: Vec<Entry> = journal
        .entries()?
        .into_iter()
        .filter(|e| !e.state.is_final())
        .collect();
    for first in pending {
        let leave = |reason: String| Pending {
            txn: first.txn.clone(),
            path: first.path.clone(),
            reason,
        };
        let Some(lock) = TxnLock::try_acquire(&journal, &first.txn)? else {
            report.pending.push(leave("running".into()));
            continue;
        };
        // Read again under the lock: it may have ended meanwhile.
        let entry = match journal.entry(&first.txn) {
            Ok(Some(e)) if !e.state.is_final() => e,
            Ok(_) => continue,
            Err(e) => {
                report.pending.push(leave(e.to_string()));
                continue;
            }
        };
        if let Some(reason) = unreachable(&entry) {
            report.pending.push(leave(reason));
            continue;
        }
        match recover_one(&journal, &entry) {
            Ok(Some(r)) => {
                tracing::info!(txn = %r.txn, path = %r.path.display(), reached = r.reached.as_str(), outcome = ?r.outcome, "recovered");
                report.recovered.push(r);
            }
            Ok(None) => report.pending.push(leave("cleanup incomplete".into())),
            Err(e) => {
                tracing::warn!(txn = %entry.txn, error = %e, "not recovered");
                report.pending.push(leave(e.to_string()));
            }
        }
        drop(lock);
    }
    journal.sweep_target_locks();
    Ok(report)
}

/// Why `e` cannot be recovered now: its target folder (or its source's, or its backup's)
/// is not there, as when its volume is not mounted.
fn unreachable(e: &Entry) -> Option<String> {
    let mut folders = vec![e.path.parent(), e.source.parent()];
    if let Some(b) = e.backup.as_deref().or(e.backup_target.as_deref()) {
        folders.push(b.parent().and_then(Path::parent));
    }
    folders
        .into_iter()
        .flatten()
        .find(|d| !d.is_dir())
        .map(|d| format!("{} is not reachable (volume not mounted?)", d.display()))
}

/// Whether the file at `path` hashes to `want` (hex).
pub(crate) fn holds(path: &Path, want: Option<&str>) -> bool {
    match (hash_file(path), want) {
        (Ok((_, h)), Some(w)) => hex(&h) == w,
        _ => false,
    }
}

/// The action for `e`, the target compared when the rename's fate is unknown.
fn action_for(e: &Entry) -> Action {
    match decide(e.kind, e.reached, exists(&e.temp)) {
        Action::CheckTarget if holds(&e.path, e.output_blake3.as_deref()) => Action::Complete,
        Action::CheckTarget => Action::RollBack,
        other => other,
    }
}

/// Recovers `e`; `None` when a rollback could not remove everything (it stays pending, with a
/// note line).
fn recover_one(journal: &Journal, e: &Entry) -> Result<Option<Recovered>> {
    let (outcome, notes) = match action_for(e) {
        Action::Complete => (Outcome::Completed, complete(journal, e)?),
        Action::RollBack | Action::CheckTarget => match roll_back(e) {
            Ok(notes) => (Outcome::RolledBack, notes),
            Err(err) => {
                note(journal, e, format!("cleanup failed: {err}"));
                return Ok(None);
            }
        },
    };
    let mut line = Line::new(&e.txn, State::Recovered);
    line.outcome = Some(outcome);
    line.notes.clone_from(&notes);
    journal.append(&line)?;
    Ok(Some(Recovered {
        txn: e.txn.clone(),
        kind: e.kind,
        path: e.path.clone(),
        reached: e.reached,
        outcome,
        notes,
    }))
}

/// Journals `text` as a note on `e` without changing its state.
fn note(journal: &Journal, e: &Entry, text: String) {
    let mut line = Line::new(&e.txn, e.reached);
    line.notes.push(text);
    if let Err(err) = journal.append(&line) {
        tracing::warn!(txn = %e.txn, error = %err, "note not journaled");
    }
}

/// After `error` stopped transaction `e` (whose lock the caller holds): rolls it back and
/// journals `failed` when the rename did not happen and every file it made is removed;
/// otherwise leaves it pending for [`recover`] (with a note). Returns whether it was rolled
/// back.
pub(crate) fn settle_failure(journal: &Journal, e: &Entry, error: &Error) -> bool {
    if action_for(e) == Action::Complete {
        note(journal, e, format!("stopped after the rename: {error}"));
        return false;
    }
    match roll_back(e) {
        Ok(notes) => {
            let mut line = Line::new(&e.txn, State::Failed);
            line.error = Some(error.to_string());
            line.notes = notes;
            if let Err(err) = journal.append(&line) {
                tracing::warn!(txn = %e.txn, error = %err, "failure not journaled");
            }
            true
        }
        Err(err) => {
            note(journal, e, format!("cleanup failed: {err}"));
            false
        }
    }
}

/// Removes what `e` created before its rename. A backup (or a file at the backup's final
/// name) is removed when empty (a placeholder) or once the target holds the original again;
/// else it is kept with a note. For a copy to a folder, an empty placeholder at the target is
/// removed.
fn roll_back(e: &Entry) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    remove_if_exists(&e.temp)?;
    if let Some(t) = &e.backup_temp {
        remove_if_exists(t)?;
    }
    if e.kind == TxnKind::ToFolder && is_placeholder(&e.path, e.output_bytes) {
        remove_if_exists(&e.path)?;
    }
    let mut backups: Vec<&Path> = Vec::new();
    for b in [e.backup.as_deref(), e.backup_target.as_deref()]
        .into_iter()
        .flatten()
    {
        if !backups.contains(&b) {
            backups.push(b);
        }
    }
    let original_in_place = e.kind == TxnKind::InPlace
        && backups.iter().any(|b| exists(b))
        && holds(&e.path, e.original_blake3.as_deref());
    for backup in backups.into_iter().filter(|b| exists(b)) {
        if is_placeholder(backup, Some(1)) || original_in_place {
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

/// Whether `path` is an empty file where a non-empty one (`expected` bytes) was going to be.
fn is_placeholder(path: &Path, expected: Option<u64>) -> bool {
    expected.is_some_and(|n| n > 0)
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == 0)
}

fn complete(journal: &Journal, e: &Entry) -> Result<Vec<String>> {
    let meta_source = match e.kind {
        TxnKind::InPlace => e
            .backup
            .as_deref()
            .or(e.backup_target.as_deref())
            .ok_or_else(|| {
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
            let mut entry = e.clone();
            if entry.backup.is_none() {
                entry.backup.clone_from(&entry.backup_target);
            }
            if let Err(err) = sidecar::write(&entry, &notes) {
                notes.push(format!("sidecar not written: {err}"));
            }
        }
        _ => {}
    }
    Ok(notes)
}

#[cfg(test)]
mod tests;
