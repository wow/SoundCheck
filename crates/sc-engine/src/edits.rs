//! Saved grid edits applied where an analysis becomes a row: in the app's session and in
//! `sc-cli plan` and `labels`, so both show the same grids and count the same rows.
//!
//! An edit applies to the audio it was made on (same decoded length, sample rate and integrated
//! loudness, so tag writes by DJ apps keep it). Its grid is the file's evidence solved with the
//! edit's overrides under the BPM range it was made with, which is the grid the user saw even
//! after the range setting changes. A confirmation counts only while that grid is still the one
//! the user confirmed: a new beat model or solver that moves it sends the row back to review.

use sc_analysis::refit::{self, Context};
use sc_core::analysis::{AnalysisRecord, Grid, GridEdit};
use sc_core::{Bpm, Result};
use sc_io::edits::{AudioIdentity, EDIT_SCHEMA, EditStore, GridPin, SavedEdit};

/// Largest tempo difference, in BPM, at which a refitted grid is still the one confirmed.
const PIN_BPM: f64 = 0.005;
/// Largest bar-1 difference, in samples, at which a refitted grid is still the one confirmed.
const PIN_SAMPLES: u64 = 1;

/// What a saved edit did to a record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditState {
    /// The grid differs from the analysis: the user's overrides were applied.
    pub edited: bool,
    /// The user confirmed this grid by ear.
    pub confirmed: bool,
}

/// The audio `record` describes.
#[must_use]
pub fn audio_of(record: &AnalysisRecord) -> AudioIdentity {
    AudioIdentity {
        frames: record.frames,
        sample_rate: record.spec.sample_rate,
        integrated: record.loudness.integrated,
    }
}

fn pin_of(grid: &Grid) -> GridPin {
    GridPin {
        bpm: grid.bpm,
        anchor: grid.anchor,
        meter: grid.meter.clone(),
    }
}

fn same_grid(pin: Option<&GridPin>, grid: Option<&Grid>) -> bool {
    match (pin, grid) {
        (Some(p), Some(g)) => {
            (p.bpm.0 - g.bpm.0).abs() <= PIN_BPM
                && p.anchor.0.abs_diff(g.anchor.0) <= PIN_SAMPLES
                && p.meter == g.meter
        }
        (None, None) => true,
        _ => false,
    }
}

/// An edit that gives no grid for a file that has one (or a file without evidence).
struct Unsolved;

/// The grid of `record` under `edit` made with `bpm_range`; `Ok(None)` for a file without a grid
/// and no edit.
fn solve(
    record: &AnalysisRecord,
    bpm_range: (Bpm, Bpm),
    edit: &GridEdit,
) -> std::result::Result<Option<Grid>, Unsolved> {
    let Some(evidence) = &record.evidence else {
        // Without evidence only "the analysis as it is" can be reproduced.
        return if edit.is_empty() {
            Ok(record.grid.clone())
        } else {
            Err(Unsolved)
        };
    };
    let ctx = Context {
        bpm_range,
        tags: &record.tags,
        sample_rate: record.spec.sample_rate,
    };
    match refit::refit(evidence, &ctx, edit) {
        Some(solved) => Ok(Some(solved.grid)),
        None if edit.is_empty() && record.grid.is_none() => Ok(None),
        None => Err(Unsolved),
    }
}

/// Replaces `record`'s grid with the grid of the edit saved for its file, when the edit was made
/// on the same audio and still gives a grid. A record without evidence (from an older cache
/// entry) takes only an edit that leaves the analysis as it is.
pub fn apply_saved(record: &mut AnalysisRecord, store: &EditStore) -> EditState {
    let Some(saved) = store.get(&record.path) else {
        return EditState::default();
    };
    if saved.audio != audio_of(record) {
        tracing::info!(file = %record.path, "grid edit made on other audio; not applied");
        return EditState::default();
    }
    let Ok(grid) = solve(record, saved.bpm_range, &saved.edit) else {
        return EditState::default();
    };
    let state = EditState {
        edited: grid != record.grid,
        confirmed: saved.confirmed && same_grid(saved.grid.as_ref(), grid.as_ref()),
    };
    if grid.is_some() {
        record.grid_skipped = None;
        record.grid = grid;
    }
    state
}

/// Saves `edit` of `record` (analysed with `bpm_range`, evidence included) with the grid it
/// gives, confirmed or not, and returns the record's state under it. An empty unconfirmed edit
/// removes the saved one.
///
/// # Errors
/// [`sc_core::Error::InvalidArgument`] when the edit is outside its limits or gives no grid on a
/// file that has one; [`sc_core::Error::Io`] when it cannot be written.
pub fn save_edit(
    store: &EditStore,
    record: &AnalysisRecord,
    bpm_range: (Bpm, Bpm),
    edit: &GridEdit,
    confirmed: bool,
) -> Result<EditState> {
    edit.validate()?;
    if edit.is_empty() && !confirmed {
        store.remove(&record.path)?;
        return Ok(EditState::default());
    }
    let grid = solve(record, bpm_range, edit).map_err(|Unsolved| {
        sc_core::Error::InvalidArgument("the edit gives no grid for this file".into())
    })?;
    store.put(&SavedEdit {
        schema: EDIT_SCHEMA,
        path: record.path.clone(),
        audio: audio_of(record),
        bpm_range,
        edit: edit.clone(),
        grid: grid.as_ref().map(pin_of),
        confirmed,
    })?;
    Ok(EditState {
        edited: grid != record.grid,
        confirmed,
    })
}
