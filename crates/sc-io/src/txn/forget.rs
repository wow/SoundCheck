//! Forgetting a pending transaction: the user gives up on one that [`super::recover`] keeps
//! leaving pending (its volume will never be mounted again, its folder was moved away).
//!
//! A `forgotten` line ends it, so recovery skips it from then on. Nothing is deleted: the
//! backup of the original, its temp copy and the transaction's temp file stay where they are,
//! and [`Forgotten::kept`] lists the ones that may still exist so the user can find them. Only
//! a transaction that has not ended can be forgotten, and not while it runs.

use std::path::{Path, PathBuf};

use sc_core::{Error, Result};

use super::fsx::exists;
use super::journal::{Journal, Line, State, TxnKind, TxnLock};

/// What [`forget`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forgotten {
    /// Transaction id.
    pub txn: String,
    /// What it did.
    pub kind: TxnKind,
    /// The file it targeted.
    pub path: PathBuf,
    /// The furthest state it had reached.
    pub reached: State,
    /// The backup of the original (made, or being made, by an in-place change), when it may
    /// still exist.
    pub backup: Option<PathBuf>,
    /// Every file the transaction made that may still exist: present, or in a folder that cannot
    /// be reached now. None of them was deleted.
    pub kept: Vec<PathBuf>,
}

/// Ends the pending transaction `txn` of the journal in `backup_root` as `forgotten`, keeping
/// every file it left.
///
/// # Errors
/// [`Error::InvalidArgument`] when no such transaction is journaled, when it has already ended
/// (done, failed, recovered or forgotten), or while it runs; [`Error::Io`] when the journal
/// cannot be read or written.
pub fn forget(backup_root: &Path, txn: &str) -> Result<Forgotten> {
    let unknown = || {
        Error::InvalidArgument(format!(
            "no change {txn} is recorded in {}",
            backup_root.display()
        ))
    };
    if !backup_root.is_dir() {
        return Err(unknown());
    }
    let journal = Journal::open(backup_root)?;
    let Some(_lock) = TxnLock::try_acquire(&journal, txn)? else {
        return Err(Error::InvalidArgument(format!(
            "change {txn} is running; it can only be forgotten once it has stopped"
        )));
    };
    let entry = journal.entry(txn)?.ok_or_else(unknown)?;
    if entry.state.is_final() {
        return Err(Error::InvalidArgument(format!(
            "change {txn} is already {}; only an unfinished change can be forgotten",
            entry.state.as_str()
        )));
    }
    let backup = entry
        .backup
        .clone()
        .or_else(|| entry.backup_target.clone())
        .filter(|b| may_exist(b));
    let mut kept: Vec<PathBuf> = Vec::new();
    let candidates = [
        Some(entry.temp.clone()),
        entry.backup_temp.clone(),
        entry.backup.clone(),
        entry.backup_target.clone(),
    ];
    for path in candidates.into_iter().flatten() {
        if may_exist(&path) && !kept.contains(&path) {
            kept.push(path);
        }
    }
    let mut line = Line::new(txn, State::Forgotten);
    line.notes = kept
        .iter()
        .map(|p| format!("kept {}", p.display()))
        .collect();
    journal.append(&line)?;
    tracing::info!(txn, path = %entry.path.display(), reached = entry.reached.as_str(), "forgotten");
    Ok(Forgotten {
        txn: entry.txn,
        kind: entry.kind,
        path: entry.path,
        reached: entry.reached,
        backup,
        kept,
    })
}

/// Whether `path` exists, or its folder cannot be reached so it cannot be known.
fn may_exist(path: &Path) -> bool {
    exists(path) || path.parent().is_some_and(|d| !d.is_dir())
}

#[cfg(test)]
mod tests;
