//! The steps after the rename (metadata, sidecar, done), which never undo the change.

use std::path::Path;
use std::time::{Duration, Instant};

use sc_core::{RenderRequest, Result};

use super::TxnReport;
use super::apply::{Plan, Prepared};
use super::crash;
use super::fsx::hex;
use super::journal::{Entry, Line, State};
use super::meta::{FileMeta, restore};
use super::sidecar::{self, Record, RenderSummary};

/// Metadata, sidecar, done: the file is in place, so nothing here undoes it; a failure leaves
/// the transaction for [`super::recover`].
pub(super) fn finish(
    plan: &Plan<'_>,
    req: &RenderRequest,
    prepared: Prepared,
    timings: &mut Vec<(State, Duration)>,
) -> Result<TxnReport> {
    let t = Instant::now();
    let restored = metadata_step(&plan.target, &plan.meta, plan.opts.keep_mtime);
    let mut line = Line::new(&plan.id, State::MetadataDone);
    line.notes.clone_from(&restored);
    // The backup's notes are on its own journal line.
    let mut notes = prepared.backup_notes.clone();
    notes.extend(restored);
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
        Ok(notes) => notes,
        Err(e) => {
            tracing::warn!(path = %target.display(), error = %e, "metadata not restored");
            vec![format!("metadata not restored: {e}")]
        }
    }
}

/// The journal entry of the finished transaction, built from what it knows.
pub(super) fn entry_of(plan: &Plan<'_>, req: &RenderRequest, p: &Prepared) -> Entry {
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
        backup_target: None,
        volume: None,
        folder: None,
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
            export: plan.opts.export.clone(),
        }),
        outcome: None,
        error: None,
        notes: Vec::new(),
        undone: false,
    }
}
