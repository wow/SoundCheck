//! The in-place and copy-to-folder transactions: preflight, temp, render, sync, verify, backup,
//! rename, metadata, sidecar, each step journaled.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use sc_core::{Error, RenderRequest, Result};

use super::crash;
use super::fsx::{
    copy_new_hashed, exists, hash_file, hex, io_err, new_txn_id, numbered, remove_if_exists,
    rename_noreplace, sync_dir, sync_path, temp_name, utc_date, utc_timestamp,
};
use super::journal::{Entry, Journal, Line, State, TxnKind, TxnLock};
use super::meta::{FileMeta, restore, snapshot};
use super::preflight::{
    Container, Source, check_in_place, check_not_rekordbox, check_space, resolve, source,
};
use super::sidecar::{self, Record, RenderSummary};
use super::verify::{Check, verify};
use super::volume::Volume;
use super::{Transaction, TxnOptions, TxnReport};
use crate::render::{self, RenderReport, check_cancel};

/// Most numbered names tried for a backup whose name is taken.
const MAX_BACKUP_SUFFIX: u32 = 10_000;

/// What a transaction writes where.
struct Plan<'a> {
    id: String,
    started_at: String,
    kind: TxnKind,
    src: Source,
    meta: FileMeta,
    target: PathBuf,
    target_dir: PathBuf,
    temp: PathBuf,
    /// In place: the backup's folder and its temp file.
    backup: Option<(PathBuf, PathBuf)>,
    opts: &'a TxnOptions,
    journal: Journal,
}

/// What the steps before the rename produced.
struct Prepared {
    report: RenderReport,
    output: (u64, [u8; 32]),
    original: (u64, [u8; 32]),
    backup: Option<PathBuf>,
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
    let plan = plan(tx, path, out_dir, req, opts)?;
    let lock = TxnLock::try_acquire(&plan.journal, &plan.id)?
        .ok_or_else(|| Error::Internal(format!("transaction {} is locked", plan.id)))?;
    plan.journal.append(&planned_line(&plan))?;
    crash::after(State::Planned);
    let mut timings = vec![(State::Planned, started.elapsed())];
    let prepared = match before_rename(tx, &plan, req, cancel, &mut timings) {
        Ok(p) => p,
        Err(e) => {
            roll_back(&plan, &e);
            return Err(e);
        }
    };
    let t = Instant::now();
    if let Err(e) = rename_into_place(&plan) {
        roll_back(&plan, &e);
        return Err(e);
    }
    sync_dir(&plan.target_dir);
    plan.journal.append(&Line::new(&plan.id, State::Renamed))?;
    crash::after(State::Renamed);
    timings.push((State::Renamed, t.elapsed()));
    let report = finish(&plan, req, prepared, &mut timings)?;
    drop(lock);
    Ok(report)
}

/// Checks the source and the destination, and decides every path.
fn plan<'a>(
    tx: &Transaction<'_>,
    path: &Path,
    out_dir: Option<&Path>,
    req: &RenderRequest,
    opts: &'a TxnOptions,
) -> Result<Plan<'a>> {
    let (full, dir, name) = resolve(path)?;
    let journal = Journal::open(&opts.backup_root)?;
    let id = new_txn_id();
    let (kind, target_dir, volume) = match out_dir {
        None => {
            let volume = check_in_place(&full, &dir, tx.volumes)?;
            (TxnKind::InPlace, dir.clone(), volume)
        }
        Some(out) => {
            let meta = std::fs::metadata(&full).map_err(|e| io_err(&full, e))?;
            if !meta.is_file() {
                return Err(Error::InvalidArgument(format!(
                    "{} is not a regular file",
                    full.display()
                )));
            }
            std::fs::create_dir_all(out).map_err(|e| io_err(out, e))?;
            let out = out.canonicalize().map_err(|e| io_err(out, e))?;
            let volume = tx.volumes.volume_of(&out)?;
            check_not_rekordbox(&out, &volume)?;
            (TxnKind::ToFolder, out, volume)
        }
    };
    let target = target_dir.join(&name);
    if kind == TxnKind::ToFolder && exists(&target) {
        return Err(Error::AlreadyExists { path: target });
    }
    let src = source(full, name, req)?;
    let meta = snapshot(&src.path)?;
    let backup = if kind == TxnKind::InPlace {
        let backup_volume = tx.volumes.volume_of(journal.root())?;
        check_space(&[(&volume, src.output_estimate), (&backup_volume, meta.len)])?;
        let dest = backup_dest(journal.root(), &volume, &src.path);
        let dest_dir = dest.parent().unwrap_or(journal.root()).to_path_buf();
        let temp = dest_dir.join(temp_name(&src.name, &id));
        Some((dest, temp))
    } else {
        check_space(&[(&volume, src.output_estimate)])?;
        None
    };
    let temp = target_dir.join(temp_name(&src.name, &id));
    Ok(Plan {
        id,
        started_at: utc_timestamp(std::time::SystemTime::now()),
        kind,
        src,
        meta,
        target,
        target_dir,
        temp,
        backup,
        opts,
        journal,
    })
}

/// `<root>/<yyyy-mm-dd>/<volume name>/<path relative to the volume>` (UTC date).
pub(crate) fn backup_dest(root: &Path, volume: &Volume, path: &Path) -> PathBuf {
    let relative = volume
        .relative(path)
        .map_or_else(|| path.strip_prefix("/").unwrap_or(path), |r| r);
    root.join(utc_date(std::time::SystemTime::now()))
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
    sync_path(&plan.temp)?;
    plan.journal
        .append(&Line::new(&plan.id, State::TempWritten))?;
    crash::after(State::TempWritten);
    timings.push((State::TempWritten, t.elapsed()));
    #[cfg(test)]
    if let Some(hook) = tx.after_temp_written {
        hook(&plan.temp);
    }
    #[cfg(not(test))]
    let _ = tx;

    let t = Instant::now();
    let output = verify(&plan.temp, &plan.target, &report, &check, cancel)?;
    let mut line = Line::new(&plan.id, State::Verified);
    line.output_blake3 = Some(hex(&output.1));
    line.output_bytes = Some(output.0);
    line.record = Some(Record {
        request: req.clone(),
        render: RenderSummary::of(&report),
    });
    let original = if plan.kind == TxnKind::ToFolder {
        let original = hash_file(&plan.src.path)?;
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

    let (original, backup) = match (&plan.backup, original) {
        (Some((dest, temp)), _) => {
            let t = Instant::now();
            let (original, backup) = back_up(plan, dest, temp)?;
            timings.push((State::BackedUp, t.elapsed()));
            (original, Some(backup))
        }
        (None, Some(original)) => (original, None),
        (None, None) => return Err(Error::Internal("no original hash".into())),
    };
    check_unchanged(&plan.src.path, &plan.meta, original.0)?;
    check_cancel(cancel)?;
    Ok(Prepared {
        report,
        output,
        original,
        backup,
    })
}

/// Copies the original to its backup temp (synced, hashed, with the original's metadata), then
/// renames it to the first free name of `dest`, `dest (2)`, ...
fn back_up(plan: &Plan<'_>, dest: &Path, temp: &Path) -> Result<((u64, [u8; 32]), PathBuf)> {
    let dir = temp.parent().unwrap_or(plan.journal.root());
    std::fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
    let original = copy_new_hashed(&plan.src.path, temp)?;
    let refused = restore(temp, &plan.meta, true)?;
    if !refused.is_empty() {
        tracing::warn!(path = %temp.display(), refused = ?refused, "backup lacks some extended attributes");
    }
    let mut n = 1;
    let backup = loop {
        let candidate = if n == 1 {
            dest.to_path_buf()
        } else {
            numbered(dest, n)
        };
        match rename_noreplace(temp, &candidate) {
            Ok(()) => break candidate,
            Err(Error::AlreadyExists { .. }) if n < MAX_BACKUP_SUFFIX => n += 1,
            Err(e) => return Err(e),
        }
    };
    sync_dir(dir);
    let mut line = Line::new(&plan.id, State::BackedUp);
    line.backup = Some(backup.clone());
    line.original_blake3 = Some(hex(&original.1));
    line.original_bytes = Some(original.0);
    if let Err(e) = plan.journal.append(&line) {
        let _ = remove_if_exists(&backup);
        return Err(e);
    }
    crash::after(State::BackedUp);
    tracing::debug!(path = %plan.src.path.display(), stage = "backup", backup = %backup.display(), "backed up");
    Ok((original, backup))
}

/// [`Error::FileChanged`] when the source's length or modification time moved since the
/// preflight, or its length differs from what was copied or hashed.
fn check_unchanged(path: &Path, before: &FileMeta, copied_len: u64) -> Result<()> {
    let now = std::fs::metadata(path).map_err(|e| io_err(path, e))?;
    let modified = now.modified().map_err(|e| io_err(path, e))?;
    if now.len() != before.len || copied_len != before.len || modified != before.modified {
        return Err(Error::FileChanged {
            path: path.to_path_buf(),
            detail: "while SoundCheck was processing it; it was left as it is. Try again".into(),
        });
    }
    Ok(())
}

fn rename_into_place(plan: &Plan<'_>) -> Result<()> {
    match plan.kind {
        TxnKind::ToFolder => rename_noreplace(&plan.temp, &plan.target),
        _ => std::fs::rename(&plan.temp, &plan.target).map_err(|e| io_err(&plan.target, e)),
    }
}

/// Removes what the transaction created before the rename and journals the failure; the
/// target is untouched.
fn roll_back(plan: &Plan<'_>, error: &Error) {
    let mut leftovers = vec![plan.temp.clone()];
    if let Some((_, backup_temp)) = &plan.backup {
        leftovers.push(backup_temp.clone());
    }
    if let Ok(Some(entry)) = plan.journal.entry(&plan.id)
        && let Some(backup) = entry.backup
    {
        leftovers.push(backup);
    }
    for path in &leftovers {
        if let Err(e) = remove_if_exists(path) {
            tracing::warn!(path = %path.display(), error = %e, "left behind after a failed transaction");
        }
    }
    let mut line = Line::new(&plan.id, State::Failed);
    line.error = Some(error.to_string());
    if let Err(e) = plan.journal.append(&line) {
        tracing::warn!(txn = %plan.id, error = %e, "failure not journaled");
    }
    tracing::info!(path = %plan.src.path.display(), txn = %plan.id, error = %error, "not processed");
}

/// Metadata, sidecar, done: the file is in place, so nothing here undoes it; a failure leaves
/// the transaction for [`super::recover`].
fn finish(
    plan: &Plan<'_>,
    req: &RenderRequest,
    prepared: Prepared,
    timings: &mut Vec<(State, Duration)>,
) -> Result<TxnReport> {
    let t = Instant::now();
    let notes = metadata_step(&plan.target, &plan.meta, plan.opts.keep_mtime);
    let mut line = Line::new(&plan.id, State::MetadataDone);
    line.notes.clone_from(&notes);
    plan.journal.append(&line)?;
    crash::after(State::MetadataDone);
    timings.push((State::MetadataDone, t.elapsed()));

    let t = Instant::now();
    let entry = entry_of(plan, req, &prepared);
    let mut line = Line::new(&plan.id, State::Done);
    let sidecar = if plan.opts.sidecar {
        match sidecar::write(&entry, &notes) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!(path = %plan.target.display(), error = %e, "sidecar not written");
                line.notes.push(format!("sidecar not written: {e}"));
                None
            }
        }
    } else {
        None
    };
    let mut notes = notes;
    notes.extend(line.notes.iter().cloned());
    plan.journal.append(&line)?;
    timings.push((State::Done, t.elapsed()));
    tracing::info!(
        path = %plan.src.path.display(),
        output = %plan.target.display(),
        txn = %plan.id,
        backup = ?prepared.backup,
        gain_db = req.gain_db,
        trim_frames = req.trim_frames,
        "processed"
    );
    Ok(TxnReport {
        txn: plan.id.clone(),
        kind: plan.kind,
        source: plan.src.path.clone(),
        output: plan.target.clone(),
        backup: prepared.backup,
        sidecar,
        render: prepared.report,
        original_blake3: prepared.original.1,
        output_blake3: prepared.output.1,
        output_bytes: prepared.output.0,
        notes,
        timings: std::mem::take(timings),
    })
}

/// Restores `meta` onto `target`; what could not be restored becomes notes.
pub(crate) fn metadata_step(target: &Path, meta: &FileMeta, keep_mtime: bool) -> Vec<String> {
    match restore(target, meta, keep_mtime) {
        Ok(refused) => refused
            .into_iter()
            .map(|n| format!("extended attribute not restored: {n}"))
            .collect(),
        Err(e) => {
            tracing::warn!(path = %target.display(), error = %e, "metadata not restored");
            vec![format!("metadata not restored: {e}")]
        }
    }
}

/// The journal entry of the finished transaction, built from what it knows.
fn entry_of(plan: &Plan<'_>, req: &RenderRequest, p: &Prepared) -> Entry {
    Entry {
        txn: plan.id.clone(),
        kind: plan.kind,
        state: State::MetadataDone,
        reached: State::MetadataDone,
        started_at: plan.started_at.clone(),
        path: plan.target.clone(),
        source: plan.src.path.clone(),
        temp: plan.temp.clone(),
        backup_temp: plan.backup.as_ref().map(|(_, t)| t.clone()),
        backup: p.backup.clone(),
        keep_mtime: plan.opts.keep_mtime,
        sidecar: plan.opts.sidecar,
        undoes: None,
        original_blake3: Some(hex(&p.original.1)),
        original_bytes: Some(p.original.0),
        output_blake3: Some(hex(&p.output.1)),
        output_bytes: Some(p.output.0),
        record: Some(Record {
            request: req.clone(),
            render: RenderSummary::of(&p.report),
        }),
        outcome: None,
        error: None,
        notes: Vec::new(),
        undone: false,
    }
}
