//! Undo: putting the backup of the newest change of a file back in its place, through the same
//! steps as a change (temp file, sync, verify, rename, metadata), journaled as a transaction of
//! kind [`TxnKind::Undo`].
//!
//! The file must still be exactly what SoundCheck wrote (same BLAKE3), so an edit made since
//! (tags written by another app) is never lost silently; the temp copy of the backup must hash
//! to the original's recorded BLAKE3. The original's metadata comes from the backup, which
//! carries it, modification time included. The sidecar is removed, or rewritten for the
//! previous change when the file had been changed more than once. The backup is kept.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::{ChangeCause, Error, Result};

use super::Transaction;
use super::crash;
use super::finish::metadata_step;
use super::fsx::{hash_file, hex, io_err, new_txn_id, sync_dir, system_copy_hashed, temp_name};
use super::journal::{Entry, Journal, Line, State, TargetLock, TxnKind, TxnLock};
use super::meta::{FileId, folder_id, snapshot};
use super::preflight::{check_in_place, check_outside_backups, check_space, resolve};
use super::recover::settle_failure;
use super::sidecar;
use super::volume::volume_identity;

/// What happened to the sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarAfterUndo {
    /// It was removed.
    Removed,
    /// It now describes the previous change, which is again the file's current state.
    Restored(PathBuf),
    /// There was none.
    Absent,
}

/// What [`Transaction::undo`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoReport {
    /// The undo's transaction id.
    pub txn: String,
    /// The transaction undone.
    pub undone: String,
    /// The file restored.
    pub path: PathBuf,
    /// The backup it was restored from (kept).
    pub backup: PathBuf,
    /// BLAKE3 of the restored file (the original's).
    pub restored_blake3: [u8; 32],
    /// What happened to the sidecar.
    pub sidecar: SidecarAfterUndo,
    /// Earlier changes of the file that are still in effect (another undo goes one further
    /// back).
    pub earlier_changes: usize,
}

/// Whether `e` is a finished in-place change of `path` that no undo has reversed.
fn undoable(e: &Entry, path: &Path) -> bool {
    e.kind == TxnKind::InPlace && e.path == path && e.completed() && !e.undone
}

pub(super) fn undo(tx: &Transaction<'_>, path: &Path, backup_root: &Path) -> Result<UndoReport> {
    let (full, dir, name) = resolve(path)?;
    let nothing = || Error::NothingToUndo { path: full.clone() };
    if !backup_root.is_dir() {
        return Err(nothing());
    }
    check_outside_backups(&full, backup_root)?;
    let volume = check_in_place(&full, &dir, tx.volumes)?;
    let journal = Journal::open(backup_root)?;
    let _target_lock = TargetLock::acquire(&journal, &full, &AtomicBool::new(false))?;
    let target = journal
        .entries()?
        .into_iter()
        .rev()
        .find(|e| undoable(e, &full))
        .ok_or_else(nothing)?;
    let backup = target
        .backup
        .clone()
        .ok_or_else(|| Error::Internal(format!("transaction {} has no backup", target.txn)))?;
    check_in_place(&full, &dir, tx.volumes)?;
    let backup_meta = snapshot(&backup)?;
    check_space(&[(&volume, backup_meta.len)])?;
    let before = FileId::read(&full)?;
    let (_, now) = hash_file(&full)?;
    if Some(hex(&now)) != target.output_blake3 || FileId::read(&full)? != before {
        return Err(Error::FileChanged {
            path: full.clone(),
            detail: format!(
                "since SoundCheck processed it, so undoing would lose those changes; the \
                 original is kept at {}",
                backup.display()
            ),
            cause: ChangeCause::SinceProcessed,
        });
    }
    let id = new_txn_id();
    let lock = TxnLock::try_acquire(&journal, &id)?
        .ok_or_else(|| Error::Internal(format!("transaction {id} is locked")))?;
    let temp = dir.join(temp_name(&name, &id));
    let mut line = Line::new(&id, State::Planned);
    line.kind = Some(TxnKind::Undo);
    line.path = Some(full.clone());
    line.source = Some(backup.clone());
    line.temp = Some(temp.clone());
    line.backup = Some(backup.clone());
    line.undoes = Some(target.txn.clone());
    line.original_blake3.clone_from(&target.original_blake3);
    line.original_bytes = target.original_bytes;
    line.keep_mtime = Some(true);
    line.sidecar = Some(target.sidecar);
    line.volume = volume_identity(&dir, true);
    line.folder = folder_id(&dir);
    journal.append(&line)?;
    crash::after(State::Planned);

    let renamed = copy_and_verify(&journal, &id, &backup, &temp, &full, &target).and_then(|h| {
        if FileId::read(&full)? != before {
            return Err(Error::FileChanged {
                path: full.clone(),
                detail: "while SoundCheck was undoing its change; it was left as it is".into(),
                cause: ChangeCause::DuringProcessing,
            });
        }
        std::fs::rename(&temp, &full).map_err(|e| io_err(&full, e))?;
        Ok(h)
    });
    let restored = match renamed {
        Ok(h) => h,
        Err(e) => {
            if let Ok(Some(entry)) = journal.entry(&id) {
                settle_failure(&journal, &entry, &e);
            }
            return Err(e);
        }
    };
    crash::point("rename");
    sync_dir(&dir);
    journal.append(&Line::new(&id, State::Renamed))?;
    crash::after(State::Renamed);
    let notes = metadata_step(&full, &backup_meta, true);
    let mut line = Line::new(&id, State::MetadataDone);
    line.notes = notes;
    journal.append(&line)?;
    crash::after(State::MetadataDone);
    let (sidecar, earlier_changes) = settle_after(&journal, &full, &target.txn)?;
    journal.append(&Line::new(&id, State::Done))?;
    drop(lock);
    tracing::info!(path = %full.display(), txn = %id, undone = %target.txn, "undone");
    Ok(UndoReport {
        txn: id,
        undone: target.txn,
        path: full,
        backup,
        restored_blake3: restored,
        sidecar,
        earlier_changes,
    })
}

/// After `undone` was undone on `path`: the sidecar settled, and how many earlier changes of
/// the file are still in effect.
fn settle_after(journal: &Journal, path: &Path, undone: &str) -> Result<(SidecarAfterUndo, usize)> {
    let entries = journal.entries()?;
    let sidecar = settle_sidecar(&entries, path, undone)?;
    let earlier = entries
        .iter()
        .filter(|e| e.txn != undone && undoable(e, path))
        .count();
    Ok((sidecar, earlier))
}

/// Copies the backup into the temp file and checks that the synced copy hashes to the
/// original's recorded hash.
fn copy_and_verify(
    journal: &Journal,
    id: &str,
    backup: &Path,
    temp: &Path,
    target: &Path,
    entry: &Entry,
) -> Result<[u8; 32]> {
    system_copy_hashed(backup, temp)?;
    journal.append(&Line::new(id, State::TempWritten))?;
    crash::after(State::TempWritten);
    let (len, hash) = hash_file(temp)?;
    if Some(hex(&hash)) != entry.original_blake3 {
        return Err(Error::VerifyFailed {
            path: target.to_path_buf(),
            detail: format!(
                "the backup {} no longer matches the original it recorded",
                backup.display()
            ),
        });
    }
    let mut line = Line::new(id, State::Verified);
    line.output_blake3 = Some(hex(&hash));
    line.output_bytes = Some(len);
    journal.append(&line)?;
    crash::after(State::Verified);
    Ok(hash)
}

/// After `undone` was undone on `path`: rewrites the sidecar for the previous change still in
/// effect, or removes it.
///
/// # Errors
/// [`Error::Io`] when the sidecar cannot be written or removed.
pub(crate) fn settle_sidecar(
    entries: &[Entry],
    path: &Path,
    undone: &str,
) -> Result<SidecarAfterUndo> {
    let previous = entries
        .iter()
        .rev()
        .find(|e| e.txn != undone && undoable(e, path));
    match previous {
        Some(prev) if prev.sidecar => {
            let notes: Vec<String> = prev
                .notes
                .iter()
                .filter(|n| !n.starts_with("sidecar"))
                .cloned()
                .collect();
            Ok(SidecarAfterUndo::Restored(sidecar::write(prev, &notes)?))
        }
        _ => Ok(if sidecar::remove(path)? {
            SidecarAfterUndo::Removed
        } else {
            SidecarAfterUndo::Absent
        }),
    }
}
