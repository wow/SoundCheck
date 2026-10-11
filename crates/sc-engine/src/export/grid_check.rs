//! GRID-CHECK: every exported file's grid compared with the written file's own analysis
//! ([`sc_core::export::GridCheck`]).
//!
//! The written file is analysed once after the write (which also refreshes its cache entry);
//! the check reads that analysis and the grid the export recorded, so it costs no second
//! analysis. The fresh grid is the detector's own: an edit carried over to the written file pins
//! the confirmed grid, which would only compare the grid with itself, so it is left out. Only
//! the edit's choices that the detector is asked to follow on the source are kept: the meter,
//! the tempo octave (as a tap at the exported tempo, which takes the fitted tempo of that
//! lattice, never the typed value) and the part of the track fitted. When the user chose which
//! beat is beat 1 (a placed bar line or a beat-1 shift), the detector cannot confirm it, so the
//! lines are compared modulo the beat instead of the bar.

use sc_core::analysis::{AnalysisRecord, Grid, GridEdit};
use sc_core::export::{
    CheckPeriod, ExportedGrid, GRID_CHECK_BPM_TOLERANCE, GRID_CHECK_OFFSET_MS, GridCheck,
    GridCheckSkip,
};
use sc_io::edits::SavedEdit;

use crate::edits::refit_record;

/// Slack for comparisons of values that are equal when computed exactly.
const EPSILON: f64 = 1e-9;

/// Compares `found` (the written file's grid, `None` when its analysis found none) with
/// `exported` at `sample_rate`: the meter, then the tempo, then the line of `found` nearest to
/// the exported bar 1, modulo `period`. An exported meter whose beats differ in length is not
/// checked.
#[must_use]
pub fn compare(
    exported: &ExportedGrid,
    found: Option<&Grid>,
    sample_rate: u32,
    period: CheckPeriod,
) -> GridCheck {
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
    let bpm_diff = found.bpm.0 - exported.bpm_exact.0;
    if bpm_diff.is_nan() || bpm_diff.abs() > GRID_CHECK_BPM_TOLERANCE + EPSILON {
        return GridCheck::BpmDiffers {
            bpm_diff,
            found: found.bpm,
        };
    }
    let pulses = match period {
        CheckPeriod::Bar => found.meter.grouping.iter().map(|&g| u32::from(g)).sum(),
        CheckPeriod::Beat => u32::from(found.meter.grouping.first().copied().unwrap_or(1)),
    };
    let length = found.samples_per_beat(sample_rate) * f64::from(pulses.max(1));
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let (bar1, anchor) = (exported.bar1.0 as f64, found.anchor.0 as f64);
    let nearest = anchor + ((bar1 - anchor) / length).round() * length;
    let offset_ms = (nearest - bar1) * 1000.0 / f64::from(sample_rate.max(1));
    if offset_ms.abs() <= GRID_CHECK_OFFSET_MS + EPSILON {
        GridCheck::Pass {
            offset_ms,
            bpm_diff,
            period,
        }
    } else {
        GridCheck::OffBy {
            offset_ms,
            bpm_diff,
            period,
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
    /// The user's edit of the source, when one applied to its grid.
    pub edit: Option<&'a SavedEdit>,
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
    let rate = record.spec.sample_rate;
    let applied = exported
        .edit
        .filter(|saved| (grid.edited || grid.confirmed) && !saved.edit.is_empty());
    let Some(saved) = applied else {
        return compare(grid, record.grid.as_ref(), rate, CheckPeriod::Bar);
    };
    let period = if saved.edit.anchor.is_some() || saved.edit.downbeat_shift != 0 {
        CheckPeriod::Beat
    } else {
        CheckPeriod::Bar
    };
    let choices = GridEdit {
        meter: Some(grid.meter.clone()),
        tempo_hint: Some(grid.bpm_exact),
        fit: saved.edit.fit,
        ..GridEdit::default()
    };
    let refit = refit_record(record, saved.bpm_range, &choices).map(|s| s.grid);
    // Without evidence (it is always kept by a fresh analysis) the analysed grid is used.
    let found = refit.as_ref().or(record.grid.as_ref());
    compare(grid, found, rate, period)
}

#[cfg(test)]
mod tests;
