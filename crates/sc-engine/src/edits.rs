//! Saved grid edits applied where an analysis becomes a row: in the app's session and in
//! `sc-cli plan` and `labels`, so both show the same grids and count the same rows.

use sc_analysis::refit::{self, Context};
use sc_core::analysis::AnalysisRecord;
use sc_io::edits::EditStore;

/// What a saved edit did to a record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditState {
    /// The grid differs from the analysis: the user's overrides were applied.
    pub edited: bool,
    /// The user confirmed this grid by ear.
    pub confirmed: bool,
}

/// Replaces `record`'s grid with the grid of the edit saved for its file, when the file is
/// unchanged since: the file's evidence solved with the edit's overrides under the BPM range
/// the edit was made with, so it is the grid the user saw. A record without evidence (from an
/// older cache entry) is left as it is and counts as neither edited nor confirmed until the
/// file is analysed again.
pub fn apply_saved(record: &mut AnalysisRecord, store: &EditStore) -> EditState {
    let Some(saved) = store.get(&record.path, record.size, record.mtime_ns) else {
        return EditState::default();
    };
    let Some(evidence) = &record.evidence else {
        return EditState::default();
    };
    let ctx = Context {
        bpm_range: saved.bpm_range,
        tags: &record.tags,
        sample_rate: record.spec.sample_rate,
    };
    let grid = refit::refit(evidence, &ctx, &saved.edit).map(|solved| solved.grid);
    let edited = grid != record.grid;
    if grid.is_some() {
        record.grid_skipped = None;
        record.grid = grid;
    }
    EditState {
        edited,
        confirmed: saved.confirmed,
    }
}
