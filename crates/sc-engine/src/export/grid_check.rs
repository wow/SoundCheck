//! GRID-CHECK: every exported file's grid compared with the written file's own analysis
//! ([`sc_core::export::GridCheck`]).
//!
//! The check is relative to the source's own analysis. When the export is planned, the
//! detector's grid of the source (the analysed grid, or for a grid the user edited the same
//! evidence solved with only the user's meter, a tap at the exported tempo for its octave, and
//! the fitted part: never the bar line placed, the beat 1 chosen or the tempo typed) gives a bar
//! line at some offset from the exported bar 1, and a tempo; the export records both
//! ([`ExportedGrid::detector_offset_ms`], [`ExportedGrid::detector_bpm`]). After the write, the
//! output's fresh analysis (made anyway, to refresh its cache entry) is solved with the same
//! choices and must give a bar line within 5 ms of the same place, modulo the bar, and a tempo
//! within 0.005 BPM of the same tempo. A nudged bar 1, a chosen beat 1 and a typed tempo are
//! thereby neutral, and a cut that moved the music by a beat still fails. For a grid the user did
//! not edit the source's analysis is the exported grid, so the relative check is the absolute
//! one: the output's bar line against the exported bar 1, its tempo against the exported tempo.
//! Each refit takes well under 5 ms; nothing is analysed twice.
//!
//! Comparing modulo the bar means a cut off by a whole bar passes: the detector cannot tell
//! which of two identical-looking downbeats is bar 1, so the check cannot either. The cut itself
//! is pinned elsewhere: the write is refused unless the output has exactly the planned frame
//! count (the source's minus the planned cut).

use sc_core::Bpm;
use sc_core::analysis::{AnalysisRecord, Grid, GridEdit};
use sc_core::export::{
    ExportedGrid, GRID_CHECK_BPM_TOLERANCE, GRID_CHECK_OFFSET_MS, GridCheck, GridCheckSkip,
};
use sc_io::edits::SavedEdit;

use crate::edits::{EditState, refit_record};

/// Slack for comparisons of values that are equal when computed exactly.
const EPSILON: f64 = 1e-9;

/// The choices both analyses are solved with: none for a grid the user did not edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    /// The overrides: the meter, a tempo tap and the fitted part, or none.
    pub edit: GridEdit,
    /// The BPM range they are solved under.
    pub bpm_range: (Bpm, Bpm),
    /// Solve the evidence under `edit` and `bpm_range`: an edit applied, even an empty one
    /// whose range (the one it was saved with) gives another grid than the analysis's. Without
    /// it the analysed grid is the detector's.
    pub refit: bool,
}

impl Choices {
    /// The choices for a source whose grid as shown is `grid`, given the user's `saved` edit and
    /// what it did (`state`), or the analysis's `bpm_range` without an edit that applied.
    #[must_use]
    pub fn of(
        grid: Option<&Grid>,
        saved: Option<&SavedEdit>,
        state: EditState,
        bpm_range: (Bpm, Bpm),
    ) -> Self {
        // An edit applied: its overrides, or an empty edit that `state.edited` says was solved
        // under its own range to another grid than the analysis's.
        let applied = saved.filter(|s| state.edited || (state.confirmed && !s.edit.is_empty()));
        match (applied, grid) {
            (Some(saved), Some(grid)) => Self {
                edit: if saved.edit.is_empty() {
                    GridEdit::default()
                } else {
                    GridEdit {
                        meter: Some(grid.meter.clone()),
                        tempo_hint: Some(grid.bpm),
                        fit: saved.edit.fit,
                        ..GridEdit::default()
                    }
                },
                bpm_range: saved.bpm_range,
                refit: true,
            },
            _ => Self {
                edit: GridEdit::default(),
                bpm_range,
                refit: false,
            },
        }
    }
}

/// The detector's grid of `record` under `choices`: its analysed grid without a refit
/// (`record.grid` must then be the analysed one, no edit applied), else its evidence refitted
/// (`None` without evidence).
#[must_use]
pub fn detector_grid(record: &AnalysisRecord, choices: &Choices) -> Option<Grid> {
    if choices.refit {
        refit_record(record, choices.bpm_range, &choices.edit).map(|s| s.grid)
    } else {
        record.grid.clone()
    }
}

/// `grid`'s bar line nearest `at` (samples) minus `at`, in samples.
fn bar_offset(grid: &Grid, at: f64, sample_rate: u32) -> f64 {
    let pulses: u32 = grid.meter.grouping.iter().map(|&g| u32::from(g)).sum();
    let bar = grid.samples_per_beat(sample_rate) * f64::from(pulses.max(1));
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let anchor = grid.anchor.0 as f64;
    anchor + ((at - anchor) / bar).round() * bar - at
}

/// `exported` with the source detector's offset and tempo recorded: `detector` is the source's
/// detector grid and `anchor` the exported grid's bar 1, both in the source's samples.
#[must_use]
pub fn with_detector(
    mut exported: ExportedGrid,
    detector: Option<&Grid>,
    anchor: sc_core::SampleIndex,
    sample_rate: u32,
) -> ExportedGrid {
    if let Some(d) = detector {
        // u64 -> f64 is exact below 2^53 samples.
        #[allow(clippy::cast_precision_loss)]
        let at = anchor.0 as f64;
        exported.detector_offset_ms =
            Some(bar_offset(d, at, sample_rate) * 1000.0 / f64::from(sample_rate.max(1)));
        exported.detector_bpm = Some(d.bpm);
    }
    exported
}

/// Compares `found` (the written file's detector grid, `None` when its analysis found none) with
/// what `exported` expects at `sample_rate`: the meter, then the tempo (the source detector's,
/// else the exported one), then the bar line of `found` nearest the expected place (the exported
/// bar 1 moved by the source detector's offset), modulo the bar. An exported meter whose beats
/// differ in length is not checked.
#[must_use]
pub fn compare(exported: &ExportedGrid, found: Option<&Grid>, sample_rate: u32) -> GridCheck {
    if !exported.meter.is_regular() {
        return GridCheck::NotChecked {
            reason: GridCheckSkip::OddMeter {
                meter: exported.meter.clone(),
            },
        };
    }
    let Some(found) = found else {
        return GridCheck::NoGridFound;
    };
    if found.meter != exported.meter {
        return GridCheck::MeterDiffers {
            found: found.meter.clone(),
        };
    }
    let expected_bpm = exported.detector_bpm.unwrap_or(exported.bpm_exact);
    let bpm_diff = found.bpm.0 - expected_bpm.0;
    if bpm_diff.is_nan() || bpm_diff.abs() > GRID_CHECK_BPM_TOLERANCE + EPSILON {
        return GridCheck::BpmDiffers {
            bpm_diff,
            found: found.bpm,
        };
    }
    let rate = f64::from(sample_rate.max(1));
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let expected =
        exported.bar1.0 as f64 + exported.detector_offset_ms.unwrap_or(0.0) * rate / 1000.0;
    let offset_ms = bar_offset(found, expected, sample_rate) * 1000.0 / rate;
    if offset_ms.abs() <= GRID_CHECK_OFFSET_MS + EPSILON {
        GridCheck::Pass {
            offset_ms,
            bpm_diff,
        }
    } else {
        GridCheck::OffBy {
            offset_ms,
            bpm_diff,
        }
    }
}

/// What the check of one written file knows about its export.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Exported<'a> {
    /// The grid the export recorded; `None` without one.
    pub grid: Option<&'a ExportedGrid>,
    /// The grid was withheld because it needed review.
    pub withheld: bool,
    /// The choices the source's detector grid was solved with.
    pub choices: &'a Choices,
}

/// The check of a written file: `record` is its fresh analysis, before any edit is applied.
pub(crate) fn check_written(record: &AnalysisRecord, exported: &Exported<'_>) -> GridCheck {
    let Some(grid) = exported.grid else {
        let reason = if exported.withheld {
            GridCheckSkip::GridWithheld
        } else {
            GridCheckSkip::NoGrid
        };
        return GridCheck::NotChecked { reason };
    };
    let found = detector_grid(record, exported.choices);
    compare(grid, found.as_ref(), record.spec.sample_rate)
}

#[cfg(test)]
mod tests;
