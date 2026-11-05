//! The row an export sends the UI ([`ExportRow`]), step by step, from the engine's events. All it
//! carries is bounded, so a row stays within its byte budget whatever a file holds.

use sc_core::export::{Cut, ExportOutcome, ExportPlan};
use sc_core::ipc::{
    ExportAction, ExportRow, ExportVerification, JobStage, MAX_ROW_NOTES_JSON_BYTES,
    MAX_STALE_TAGS_JSON_BYTES, bounded_strings,
};
use sc_io::render::{BlockFate, RenderReport};
use sc_io::txn::{TxnKind, TxnReport};

use super::ProcessDone;
use super::wording::{GRID_WITHHELD, notice_text, skip_note, xml_only_text};

/// What the plan says the user should know, in words, for the row's notes.
fn plan_notes(plan: &ExportPlan) -> Vec<String> {
    let notices = plan.notices.iter().map(|n| notice_text(*n));
    notices
        .chain(plan.grid_withheld.then(|| GRID_WITHHELD.to_owned()))
        .collect()
}

/// A file planned, its write starting; `revertible`: an earlier change of it is in effect.
pub(crate) fn planned(plan: &ExportPlan, revertible: bool) -> Box<ExportRow> {
    Box::new(ExportRow {
        stage: JobStage::Processing,
        action: ExportAction::Write {
            gain_db: plan.gain_db,
            cut: plan.cut,
        },
        positions_shift: matches!(plan.cut, Cut::Cut { .. }),
        grid_withheld: plan.grid_withheld,
        notices: plan.notices.clone(),
        verification: None,
        grid_check: None,
        edit_carried: false,
        revertible,
        exported_with: None,
        notes: bounded_strings(plan_notes(plan), MAX_ROW_NOTES_JSON_BYTES),
    })
}

/// A file not written, with why in words; `revertible`: an earlier change of it is in effect.
pub(crate) fn skipped(outcome: &ExportOutcome, revertible: bool) -> Box<ExportRow> {
    let (action, notices, grid_withheld, why) = match outcome {
        ExportOutcome::XmlOnly { reason } => (
            ExportAction::XmlOnly { reason: *reason },
            Vec::new(),
            false,
            vec![xml_only_text(*reason)],
        ),
        ExportOutcome::Skip { reason } => (
            ExportAction::Skip { reason: *reason },
            Vec::new(),
            false,
            vec![skip_note(*reason)],
        ),
        // A batch reports a written file through its other events.
        ExportOutcome::Write { plan } => (
            ExportAction::Write {
                gain_db: plan.gain_db,
                cut: plan.cut,
            },
            plan.notices.clone(),
            plan.grid_withheld,
            plan_notes(plan),
        ),
    };
    Box::new(ExportRow {
        stage: JobStage::Skipped,
        action,
        positions_shift: false,
        grid_withheld,
        notices,
        verification: None,
        grid_check: None,
        edit_carried: false,
        revertible,
        exported_with: None,
        notes: bounded_strings(why, MAX_ROW_NOTES_JSON_BYTES),
    })
}

/// What was verified of the render.
fn verification(render: &RenderReport) -> ExportVerification {
    let count = |fate| u32::try_from(render.count(fate)).unwrap_or(u32::MAX);
    let tags_added = render
        .tag_edit
        .as_ref()
        .map(|e| e.replaced.saturating_add(e.appended))
        .or_else(|| {
            render
                .vorbis_edit
                .as_ref()
                .map(|e| e.replaced.saturating_add(e.appended))
        })
        .unwrap_or(0);
    ExportVerification {
        blocks: u32::try_from(render.blocks.len()).unwrap_or(u32::MAX),
        carried: count(BlockFate::Carried),
        patched: count(BlockFate::Patched),
        edited: count(BlockFate::Edited),
        replaced: count(BlockFate::Replaced),
        dropped: count(BlockFate::Dropped),
        tags_added,
        frames_in: render.frames_in,
        frames_out: render.frames_out,
        bits_out: render.bits_out,
        exact: render.exact,
        dithered: render.dithered,
        stale_loudness_tags: bounded_strings(
            render.stale_loudness_tags.iter().cloned(),
            MAX_STALE_TAGS_JSON_BYTES,
        ),
    }
}

/// A file in place and verified, its check starting.
pub(crate) fn written(row: Box<ExportRow>, report: &TxnReport) -> Box<ExportRow> {
    let not_added = report
        .render
        .tags_not_added
        .as_ref()
        .map(|why| format!("tags not added: {why}"));
    Box::new(ExportRow {
        stage: JobStage::Writing,
        positions_shift: report.render.trim_frames > 0,
        verification: Some(verification(&report.render)),
        revertible: row.revertible || report.kind == TxnKind::InPlace,
        notes: bounded_strings(
            row.notes
                .iter()
                .cloned()
                .chain(not_added)
                .chain(report.notes.iter().cloned()),
            MAX_ROW_NOTES_JSON_BYTES,
        ),
        ..*row
    })
}

/// A file written and checked.
pub(crate) fn done(row: Box<ExportRow>, done: &ProcessDone) -> Box<ExportRow> {
    let notes = bounded_strings(
        row.notes.iter().chain(&done.notes).cloned(),
        MAX_ROW_NOTES_JSON_BYTES,
    );
    Box::new(ExportRow {
        stage: JobStage::Done,
        grid_check: Some(done.grid_check.clone()),
        edit_carried: done.edit_carried,
        notes,
        ..*row
    })
}

#[cfg(test)]
mod tests;
