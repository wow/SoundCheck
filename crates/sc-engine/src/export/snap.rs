//! Planning one file's export from the head cut the renderer makes: [`plan_export`], then the
//! renderer's snap of the planned cut ([`sc_io::render::snap_head_cut`], which decodes only the
//! frames up to the cut), then [`plan_snapped_cut`]. The snap is made once, here; the render is
//! then asked for exactly the snapped cut, because snapping it again may move it further back.
//! Tested with the planner (`plan/tests/snapped.rs`).

use std::path::Path;

use sc_core::Result;
use sc_core::export::{ExportOutcome, ExportSettings};

use super::plan::{ExportInput, plan_export, plan_snapped_cut};

/// What exporting does with the file at `path` (whose analysis and source `input` describe)
/// under `settings`, with every value that depends on the head cut planned from the cut the
/// renderer makes ([`sc_core::export::ExportPlan::trim_snapped_from`] set). A file that is
/// not cut is planned without reading it.
///
/// # Errors
/// What [`sc_io::render::snap_head_cut`] returns for a file it cannot read up to the cut
/// (`UnsupportedFormat`, `Corrupt`, `Io`, `InvalidArgument` when the file is now shorter than
/// the cut).
pub fn plan_export_snapped(
    path: &Path,
    input: &ExportInput<'_>,
    settings: &ExportSettings,
) -> Result<ExportOutcome> {
    let outcome = plan_export(input, settings);
    let ExportOutcome::Write { plan } = &outcome else {
        return Ok(outcome);
    };
    if plan.trim_frames == 0 {
        return Ok(outcome);
    }
    let snapped = sc_io::render::snap_head_cut(path, plan.trim_frames)?;
    let plan = plan_snapped_cut(input, settings, plan, snapped)?;
    Ok(ExportOutcome::Write { plan })
}
