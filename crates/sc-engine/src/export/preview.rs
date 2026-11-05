//! What exporting would do with one analysed file, for the confirm step: the same
//! [`plan_export_snapped`] the job plans with, on the tags the file holds now, turned into the row
//! the job would show, with the refusals of a write in place said here too. Nothing is written.

use std::path::Path;

use sc_core::Result;
use sc_core::analysis::AnalysisRecord;
use sc_core::export::{ExportOutcome, ExportSettings, ExportSkip};
use sc_core::ipc::ExportRow;
use sc_core::plan::DecideSettings;

use super::plan::{ExportInput, ExportSource};
use super::process::gate_folder;
use super::row;
use super::snap::plan_export_snapped;
use crate::decide::decide;
use crate::edits::EditState;
use crate::txn::FolderNames;

/// One file to preview: its analysis with the grid as shown, and how it would be exported.
#[derive(Debug, Clone, Copy)]
pub struct PreviewInput<'a> {
    /// The file.
    pub path: &'a Path,
    /// Its analysis with the grid as shown.
    pub record: &'a AnalysisRecord,
    /// The state of that grid.
    pub edit: EditState,
    /// The loudness settings.
    pub decide: &'a DecideSettings,
    /// The export settings.
    pub settings: &'a ExportSettings,
    /// The folder copies go to.
    pub out_dir: Option<&'a Path>,
    /// An earlier change of the file is in effect.
    pub revertible: bool,
    /// Where backups go: the transaction refuses a file already inside.
    pub backup_root: &'a Path,
}

/// The row exporting the file would show, and whether its write replaces an earlier copy. The
/// source is not hashed, which only the record written into the file needs. `names` holds the names
/// earlier rows of the batch write under, for an export to a folder: only a row that plans a write
/// takes one, so call this in list order.
///
/// # Errors
/// What [`plan_export_snapped`] returns for a file whose start cannot be read up to the cut; what
/// reading a file of that name in the folder returns; what a write in place of this file would be
/// refused with ([`sc_io::txn::check_writable_in_place`]).
pub fn preview_row(
    input: &PreviewInput<'_>,
    names: Option<&mut FolderNames>,
) -> Result<(Box<ExportRow>, bool)> {
    let source = ExportSource::read(input.path);
    let decided = decide(
        input.record,
        source.codec,
        input.decide,
        input.edit.confirmed,
    );
    let planning = ExportInput {
        record: input.record,
        plan: &decided,
        decide: input.decide,
        source: &source,
        grid_confirmed: input.edit.confirmed,
    };
    let outcome = plan_export_snapped(input.path, &planning, input.settings)?;
    let (outcome, replaces) = gate_folder(outcome, input.path, input.out_dir, false)?;
    let writes = matches!(outcome, ExportOutcome::Write { .. });
    if writes && input.out_dir.is_none() {
        sc_io::txn::check_writable_in_place(input.path, input.backup_root)?;
    }
    let name_taken = writes
        && names
            .zip(input.path.file_name())
            .is_some_and(|(names, name)| !names.claim(name));
    let (outcome, replaces) = if name_taken {
        let reason = ExportSkip::SameNameInBatch;
        (ExportOutcome::Skip { reason }, false)
    } else {
        (outcome, replaces)
    };
    Ok((
        match outcome {
            ExportOutcome::Write { plan } => row::planned(&plan, input.revertible),
            outcome => row::skipped(&outcome, input.revertible),
        },
        replaces,
    ))
}
