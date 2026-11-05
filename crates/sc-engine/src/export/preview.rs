//! What exporting would do with one analysed file, for the confirm step: the same
//! [`plan_export_snapped`] the job plans with, on the tags the file holds now, turned into the row
//! the job would show. Nothing is written.

use std::path::Path;

use sc_core::Result;
use sc_core::analysis::AnalysisRecord;
use sc_core::export::{ExportOutcome, ExportSettings};
use sc_core::ipc::ExportRow;
use sc_core::plan::DecideSettings;

use super::plan::{ExportInput, ExportSource};
use super::row;
use super::snap::plan_export_snapped;
use crate::decide::decide;
use crate::edits::EditState;

/// The row exporting the file at `path` would show. `record` is its analysis with the grid as
/// shown and `edit` the state of that grid. The source is not hashed, which only the record written
/// into the file needs.
///
/// # Errors
/// What [`plan_export_snapped`] returns for a file whose start cannot be read up to the cut.
pub fn preview_row(
    path: &Path,
    record: &AnalysisRecord,
    edit: EditState,
    decide_settings: &DecideSettings,
    settings: &ExportSettings,
    revertible: bool,
) -> Result<Box<ExportRow>> {
    let source = ExportSource::read(path);
    let decided = decide(record, source.codec, decide_settings, edit.confirmed);
    let input = ExportInput {
        record,
        plan: &decided,
        decide: decide_settings,
        source: &source,
        grid_confirmed: edit.confirmed,
    };
    Ok(match plan_export_snapped(path, &input, settings)? {
        ExportOutcome::Write { plan } => row::planned(&plan, revertible),
        outcome => row::skipped(&outcome, revertible),
    })
}
