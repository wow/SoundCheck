//! The in-place and copy-to-folder transactions: checks, temp, render, sync, verify, backup,
//! rename; [`super::finish`] does the rest. Each step is journaled.
//!
//! Nothing is created before every refusal check has passed. Then the backup root and the
//! per-target lock are created and the target is locked for the whole transaction, so two
//! transactions on one file run one after the other; the in-place checks run again under the
//! lock and the file's identity ([`FileId`]: device, inode, length, modification and change
//! time) is read. The identity must be unchanged before the backup copy, after it and right
//! before the rename, so a file edited during processing (even with its length and
//! modification time put back) is left as it is ([`Error::FileChanged`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use sc_core::{ChangeCause, Error, RenderRequest, Result};

use super::crash;
use super::finish::finish;
use super::fsx::{
    exists, hash_file, hex, io_err, local_date, new_txn_id, numbered, rename_noreplace, sync_dir,
    sync_path, system_copy_hashed, temp_name, utc_timestamp,
};
use super::journal::{Journal, Line, State, TargetLock, TxnKind, TxnLock};
use super::meta::{FileId, FileMeta, folder_id, restore, snapshot};
use super::preflight::{
    Container, Source, check_in_place, check_not_rekordbox, check_outside_backups, check_space,
    name_in_dir, nearest_existing, resolve, source,
};
use super::recover::settle_failure;
use super::sidecar::{Record, RenderSummary};
use super::verify::{Check, verify};
use super::volume::{Volume, volume_identity};
use super::{Transaction, TxnOptions, TxnReport};
use crate::render::{self, RenderReport, check_cancel};

/// What starts each note about metadata the backup lacks.
pub(crate) const BACKUP_NOTE_PREFIX: &str = "backup: ";

/// Most numbered names tried for a backup whose name is taken.
const MAX_BACKUP_SUFFIX: u32 = 10_000;

/// What the checks found, before anything is created.
struct Checked {
    kind: TxnKind,
    src: Source,
    target: PathBuf,
    target_dir: PathBuf,
    /// The target folder does not exist yet.
    create_dir: bool,
    /// The source's identity when it was checked, before waiting for the target lock.
    source_id: FileId,
    volume: Volume,
}

/// What a transaction writes where.
pub(super) struct Plan<'a> {
    pub id: String,
    pub started_at: String,
    pub kind: TxnKind,
    pub src: Source,
    pub meta: FileMeta,
    pub target: PathBuf,
    pub target_dir: PathBuf,
    pub temp: PathBuf,
    /// In place: the backup's first-choice path and its temp file.
    pub backup: Option<(PathBuf, PathBuf)>,
    pub opts: &'a TxnOptions,
    pub journal: Journal,
}

/// What the steps before the rename produced.
pub(super) struct Prepared {
    pub report: RenderReport,
    pub output: (u64, [u8; 32]),
    pub original: (u64, [u8; 32]),
    pub backup: Option<PathBuf>,
    /// What the backup lacks of the original's metadata.
    pub backup_notes: Vec<String>,
}

/// What [`back_up`] made.
struct BackedUp {
    /// Length and BLAKE3 of the original (and the backup).
    original: (u64, [u8; 32]),
    /// The backup's final path.
    path: PathBuf,
    /// What the backup lacks of the original's metadata (`backup: ...`).
    notes: Vec<String>,
}

/// Runs an in-place (`out_dir` `None`) or copy-to-folder transaction.
pub(super) fn apply(
    tx: &Transaction<'_>,
    path: &Path,
    out_dir: Option<&Path>,
    req: &RenderRequest,
    opts: &TxnOptions,
    cancel: &AtomicBool,
) -> Result<TxnReport> {
    let started = Instant::now();
    let checked = check(tx, path, out_dir, req, opts)?;
    let journal = Journal::open(&opts.backup_root)?;
    if checked.create_dir {
        std::fs::create_dir_all(&checked.target_dir).map_err(|e| io_err(&checked.target_dir, e))?;
    }
    #[cfg(test)]
    if let Some(hook) = tx.hooks.after_check {
        hook();
    }
    let _target_lock = TargetLock::acquire(&journal, &checked.target, cancel)?;
    let plan = plan(tx, checked, journal, opts)?;
    let lock = TxnLock::try_acquire(&plan.journal, &plan.id)?
        .ok_or_else(|| Error::Internal(format!("transaction {} is locked", plan.id)))?;
    plan.journal.append(&planned_line(&plan))?;
    crash::after(State::Planned);
    let mut timings = vec![(State::Planned, started.elapsed())];
    let prepared = match before_rename(tx, &plan, req, cancel, &mut timings) {
        Ok(p) => p,
        Err(e) => return Err(fail(&plan, e)),
    };
    let t = Instant::now();
    if let Err(e) = rename_into_place(tx, &plan) {
        return Err(fail(&plan, e));
    }
    crash::point("rename");
    sync_dir(&plan.target_dir);
    plan.journal.append(&Line::new(&plan.id, State::Renamed))?;
    crash::after(State::Renamed);
    timings.push((State::Renamed, t.elapsed()));
    let report = finish(&plan, req, prepared, &mut timings)?;
    drop(lock);
    Ok(report)
}

/// Every refusal check, with nothing created: the source, the destination (or its nearest
/// existing folder), the backup root, the container and free space.
fn check(
    tx: &Transaction<'_>,
    path: &Path,
    out_dir: Option<&Path>,
    req: &RenderRequest,
    opts: &TxnOptions,
) -> Result<Checked> {
    let (full, dir, name) = resolve(path)?;
    check_outside_backups(&full, &opts.backup_root)?;
    let (kind, target_dir, create_dir, volume) = match out_dir {
        None => {
            let volume = check_in_place(&full, &dir, tx.volumes)?;
            (TxnKind::InPlace, dir, false, volume)
        }
        Some(out) => {
            let meta = std::fs::metadata(&full).map_err(|e| io_err(&full, e))?;
            if !meta.is_file() {
                return Err(Error::InvalidArgument(format!(
                    "{} is not a regular file",
                    full.display()
                )));
            }
            let (base, below) = nearest_existing(out)?;
            let volume = tx.volumes.volume_of(&base)?;
            let target_dir = base.join(&below);
            check_not_rekordbox(&target_dir, &volume)?;
            let create = !below.as_os_str().is_empty();
            (TxnKind::ToFolder, target_dir, create, volume)
        }
    };
    // A copy is named after the path the user gave, spelled as its folder spells it (with
    // hard links, the resolved name may be another link's).
    let out_name = match (kind, path.parent(), path.file_name()) {
        (TxnKind::ToFolder, Some(given_dir), Some(given)) => {
            let given_dir = if given_dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                given_dir
            };
            name_in_dir(given_dir, given, FileId::read(&full)?.ino)
        }
        _ => name.clone(),
    };
    let target = target_dir.join(&out_name);
    if kind == TxnKind::ToFolder {
        check_outside_backups(&target, &opts.backup_root)?;
        if exists(&target) {
            return Err(Error::AlreadyExists { path: target });
        }
    }
    let source_id = FileId::read(&full)?;
    let src_len = source_id.len;
    let src = source(full, name, req)?;
    if kind == TxnKind::InPlace {
        let (root, _) = nearest_existing(&opts.backup_root)?;
        let backup_volume = tx.volumes.volume_of(&root)?;
        check_space(&[(&volume, src.output_estimate), (&backup_volume, src_len)])?;
    } else {
        check_space(&[(&volume, src.output_estimate)])?;
    }
    Ok(Checked {
        kind,
        src,
        target,
        target_dir,
        create_dir,
        source_id,
        volume,
    })
}

/// Under the target lock: the in-place checks again (the file may have changed while this
/// waited), the metadata and identity, and every path.
fn plan<'a>(
    tx: &Transaction<'_>,
    c: Checked,
    journal: Journal,
    opts: &'a TxnOptions,
) -> Result<Plan<'a>> {
    if c.kind == TxnKind::InPlace {
        check_in_place(&c.src.path, &c.target_dir, tx.volumes)?;
    } else if exists(&c.target) {
        return Err(Error::AlreadyExists { path: c.target });
    }
    let meta = snapshot(&c.src.path)?;
    // Another transaction may have changed the file while this one waited for the lock: it
    // must not render that one's output again.
    if meta.id != c.source_id {
        return Err(Error::FileChanged {
            path: c.src.path,
            detail: "while SoundCheck was waiting to process it (another change of it ran \
                     first); it was left as it is"
                .into(),
            cause: ChangeCause::OtherChangeFirst,
        });
    }
    let id = new_txn_id();
    let backup = (c.kind == TxnKind::InPlace).then(|| {
        let dest = backup_dest(journal.root(), &c.volume, &c.src.path);
        let dest_dir = dest.parent().unwrap_or(journal.root()).to_path_buf();
        let temp = dest_dir.join(temp_name(&c.src.name, &id));
        (dest, temp)
    });
    let temp = c.target_dir.join(temp_name(&c.src.name, &id));
    Ok(Plan {
        id,
        started_at: utc_timestamp(std::time::SystemTime::now()),
        kind: c.kind,
        src: c.src,
        meta,
        target: c.target,
        target_dir: c.target_dir,
        temp,
        backup,
        opts,
        journal,
    })
}

/// `<root>/<yyyy-mm-dd>/<volume name>/<path relative to the volume>` (local date).
pub(crate) fn backup_dest(root: &Path, volume: &Volume, path: &Path) -> PathBuf {
    let relative = volume
        .relative(path)
        .map_or_else(|| path.strip_prefix("/").unwrap_or(path), |r| r);
    root.join(local_date(std::time::SystemTime::now()))
        .join(&volume.name)
        .join(relative)
}

fn planned_line(plan: &Plan<'_>) -> Line {
    let mut line = Line::new(&plan.id, State::Planned);
    line.at.clone_from(&plan.started_at);
    line.kind = Some(plan.kind);
    line.path = Some(plan.target.clone());
    line.source = Some(plan.src.path.clone());
    line.temp = Some(plan.temp.clone());
    line.backup_temp = plan.backup.as_ref().map(|(_, t)| t.clone());
    line.keep_mtime = Some(plan.opts.keep_mtime);
    line.sidecar = Some(plan.opts.sidecar);
    line.volume = volume_identity(&plan.target_dir, true);
    line.folder = folder_id(&plan.target_dir);
    line
}

/// Render, sync, verify, back up: everything that can still be undone by deleting files.
fn before_rename(
    tx: &Transaction<'_>,
    plan: &Plan<'_>,
    req: &RenderRequest,
    cancel: &AtomicBool,
    timings: &mut Vec<(State, Duration)>,
) -> Result<Prepared> {
    let t = Instant::now();
    let (report, check) = match plan.src.container {
        Container::Iff { wave } => (
            render::apply_iff(&plan.src.path, &plan.temp, req, cancel)?,
            Check::Iff { wave },
        ),
        Container::Flac => {
            let (report, check) =
                render::render_flac_unverified(&plan.src.path, &plan.temp, req, cancel)?;
            (report, Check::Flac(check))
        }
    };
    check_planned(plan, &report)?;
    sync_path(&plan.temp)?;
    plan.journal
        .append(&Line::new(&plan.id, State::TempWritten))?;
    crash::after(State::TempWritten);
    timings.push((State::TempWritten, t.elapsed()));
    #[cfg(test)]
    if let Some(hook) = tx.hooks.after_temp_written {
        hook(&plan.temp, &plan.src.path);
    }
    #[cfg(not(test))]
    let _ = tx;

    let t = Instant::now();
    let output = verify(
        &plan.temp,
        &plan.target,
        &plan.src.path,
        &report,
        &check,
        cancel,
    )?;
    let mut line = Line::new(&plan.id, State::Verified);
    line.output_blake3 = Some(hex(&output.1));
    line.output_bytes = Some(output.0);
    line.record = Some(Record {
        request: req.clone(),
        render: RenderSummary::of(&report),
        export: plan.opts.export.clone(),
    });
    let folder_original = if plan.kind == TxnKind::ToFolder {
        let original = hash_file(&plan.src.path)?;
        check_unchanged(&plan.src.path, &plan.meta, original.0)?;
        line.original_blake3 = Some(hex(&original.1));
        line.original_bytes = Some(original.0);
        Some(original)
    } else {
        None
    };
    plan.journal.append(&line)?;
    crash::after(State::Verified);
    timings.push((State::Verified, t.elapsed()));
    check_cancel(cancel)?;

    let (original, backup, backup_notes) = match (&plan.backup, folder_original) {
        (Some((dest, temp)), _) => {
            let t = Instant::now();
            let b = back_up(plan, dest, temp)?;
            timings.push((State::BackedUp, t.elapsed()));
            (b.original, Some(b.path), b.notes)
        }
        (None, Some(original)) => (original, None, Vec::new()),
        (None, None) => return Err(Error::Internal("no original hash".into())),
    };
    check_cancel(cancel)?;
    Ok(Prepared {
        report,
        output,
        original,
        backup,
        backup_notes,
    })
}

/// For an export, [`Error::VerifyFailed`] unless the render cut what the export planned and
/// wrote the frame count it expects (Library: the source's; Prepare: the source's minus the
/// cut), so nothing of another length replaces anything.
fn check_planned(plan: &Plan<'_>, report: &RenderReport) -> Result<()> {
    let Some(export) = &plan.opts.export else {
        return Ok(());
    };
    let planned = &export.plan;
    if report.frames_out != planned.expect_frames || report.trim_frames != planned.trim_frames {
        return Err(Error::VerifyFailed {
            path: plan.target.clone(),
            detail: format!(
                "{} frames written after a {}-frame cut; the export planned {} after {}",
                report.frames_out, report.trim_frames, planned.expect_frames, planned.trim_frames
            ),
        });
    }
    Ok(())
}

/// Copies the original to its backup temp with the system's copy (data, extended attributes,
/// the whole resource fork; synced, hashed by reading it back), gives it the original's
/// dates and mode, then renames it to the first free name of `dest`, `dest (2)`, ...,
/// journaling each name before trying it.
fn back_up(plan: &Plan<'_>, dest: &Path, temp: &Path) -> Result<BackedUp> {
    let dir = temp.parent().unwrap_or(plan.journal.root());
    std::fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
    check_unchanged(&plan.src.path, &plan.meta, plan.meta.len)?;
    let original = system_copy_hashed(&plan.src.path, temp)?;
    check_unchanged(&plan.src.path, &plan.meta, original.0)?;
    let notes: Vec<String> = restore(temp, &plan.meta, true)?
        .into_iter()
        .map(|n| format!("{BACKUP_NOTE_PREFIX}{n}"))
        .collect();
    if !notes.is_empty() {
        tracing::warn!(path = %temp.display(), notes = ?notes, "backup lacks some metadata");
    }
    let mut n = 1;
    let backup = loop {
        let candidate = if n == 1 {
            dest.to_path_buf()
        } else {
            numbered(dest, n)
        };
        if !exists(&candidate) {
            let mut line = Line::new(&plan.id, State::Verified);
            line.backup_target = Some(candidate.clone());
            line.original_blake3 = Some(hex(&original.1));
            line.original_bytes = Some(original.0);
            plan.journal.append(&line)?;
            crash::point("backup_named");
            match rename_noreplace(temp, &candidate) {
                Ok(()) => break candidate,
                Err(Error::AlreadyExists { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        if n >= MAX_BACKUP_SUFFIX {
            return Err(Error::AlreadyExists { path: candidate });
        }
        n += 1;
    };
    sync_dir(dir);
    let mut line = Line::new(&plan.id, State::BackedUp);
    line.backup = Some(backup.clone());
    line.original_blake3 = Some(hex(&original.1));
    line.original_bytes = Some(original.0);
    line.notes.clone_from(&notes);
    plan.journal.append(&line)?;
    crash::after(State::BackedUp);
    tracing::debug!(path = %plan.src.path.display(), stage = "backup", backup = %backup.display(), "backed up");
    Ok(BackedUp {
        original,
        path: backup,
        notes,
    })
}

/// [`Error::FileChanged`] unless the file at `path` still has the identity read under the
/// lock and `read_len` (what was copied or hashed) is its length.
fn check_unchanged(path: &Path, before: &FileMeta, read_len: u64) -> Result<()> {
    if FileId::read(path)? != before.id || read_len != before.len {
        return Err(Error::FileChanged {
            path: path.to_path_buf(),
            detail: "while SoundCheck was processing it; it was left as it is".into(),
            cause: ChangeCause::DuringProcessing,
        });
    }
    Ok(())
}

fn rename_into_place(tx: &Transaction<'_>, plan: &Plan<'_>) -> Result<()> {
    check_unchanged(&plan.src.path, &plan.meta, plan.meta.len)?;
    #[cfg(test)]
    if let Some(hook) = tx.hooks.before_rename {
        hook(&plan.temp, &plan.target);
    }
    #[cfg(not(test))]
    let _ = tx;
    match plan.kind {
        TxnKind::ToFolder => rename_noreplace(&plan.temp, &plan.target),
        _ => std::fs::rename(&plan.temp, &plan.target).map_err(|e| io_err(&plan.target, e)),
    }
}

/// Settles a transaction `error` stopped (rolled back and `failed`, or left pending for
/// recovery when the rename may have happened or cleanup failed); returns the error.
fn fail(plan: &Plan<'_>, error: Error) -> Error {
    match plan.journal.entry(&plan.id) {
        Ok(Some(entry)) => {
            let rolled_back = settle_failure(&plan.journal, &entry, &error);
            tracing::info!(path = %plan.src.path.display(), txn = %plan.id, error = %error, rolled_back, "not processed");
        }
        Ok(None) => tracing::warn!(txn = %plan.id, "failed transaction not in the journal"),
        Err(e) => {
            tracing::warn!(txn = %plan.id, error = %e, "journal not readable after a failure");
        }
    }
    error
}

#[cfg(test)]
mod tests;
