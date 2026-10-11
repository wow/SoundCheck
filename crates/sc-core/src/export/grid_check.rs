//! The self-check of an exported file's grid: the written file analysed afresh, its grid
//! compared with the grid the export recorded ([`super::ExportedGrid`]).
//!
//! It passes when the fresh tempo is within [`GRID_CHECK_BPM_TOLERANCE`] of the exported one
//! and a bar line of the fresh grid lies within [`GRID_CHECK_OFFSET_MS`] of the exported bar 1
//! (after a Prepare cut, the lead), compared modulo the bar: bar 1 of the fresh grid may be a
//! whole bar later and still be the same downbeat. When the user chose which beat is beat 1, the
//! detector cannot confirm that choice, so the phase is compared modulo the beat instead.
//!
//! It is the same detector checking its own result on the written audio, so a pass shows that
//! the cut, the gain and the encoding kept the grid where the export put it; it is not proof
//! that another application's analysis agrees.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::analysis::Meter;
use crate::units::Bpm;

/// Largest difference between the fresh tempo and the exported one that passes, BPM.
pub const GRID_CHECK_BPM_TOLERANCE: f64 = 0.005;

/// Largest distance of the fresh grid's nearest bar line from the exported bar 1 that passes,
/// milliseconds.
pub const GRID_CHECK_OFFSET_MS: f64 = 5.0;

/// What a line of the exported grid is compared modulo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum CheckPeriod {
    /// The bar: the detector's bar 1 must be the exported one.
    Bar,
    /// The beat: the user chose which beat is beat 1, which the detector cannot confirm.
    Beat,
}

/// Why an exported file's grid was not checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GridCheckSkip {
    /// No grid was exported: the file has none (loudness only, or no beats found).
    NoGrid,
    /// The grid needed review and was not confirmed, so it was withheld from the export.
    GridWithheld,
    /// A meter whose beats are not all the same length (9/8 as 2+2+2+3, 7/8, ...): the
    /// detector's grids in these meters are not yet reliable enough to check against.
    OddMeter {
        /// The exported meter.
        meter: Meter,
    },
    /// The file was not written: only the rekordbox XML carries its grid (MP3, AAC).
    XmlOnly,
    /// The written file's analysis did not finish (cancelled, or it failed; see the notes).
    NotAnalysed,
}

/// The result of checking an exported file's grid against the written file's own analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "result",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GridCheck {
    /// The tempo and the bar line agree.
    Pass {
        /// The fresh grid's nearest line minus the exported bar 1, milliseconds.
        offset_ms: f64,
        /// The fresh tempo minus the exported one, BPM.
        bpm_diff: f64,
        /// What the lines were compared modulo.
        period: CheckPeriod,
    },
    /// The tempo agrees, but the nearest line is more than [`GRID_CHECK_OFFSET_MS`] away.
    OffBy {
        /// The fresh grid's nearest line minus the exported bar 1, milliseconds.
        offset_ms: f64,
        /// The fresh tempo minus the exported one, BPM.
        bpm_diff: f64,
        /// What the lines were compared modulo.
        period: CheckPeriod,
    },
    /// The tempo differs by more than [`GRID_CHECK_BPM_TOLERANCE`].
    BpmDiffers {
        /// The fresh tempo minus the exported one, BPM.
        bpm_diff: f64,
        /// The fresh tempo.
        found: Bpm,
    },
    /// The fresh grid has another meter, so its bar lines cannot be compared.
    MeterDiffers {
        /// The fresh meter.
        found: Meter,
    },
    /// The written file's own analysis found no grid.
    NoGridFound,
    /// Not checked.
    NotChecked {
        /// Why.
        reason: GridCheckSkip,
    },
}

impl GridCheck {
    /// Whether the check ran and agreed.
    #[must_use]
    pub fn passed(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    /// Whether the check ran and disagreed.
    #[must_use]
    pub fn failed(&self) -> bool {
        !self.passed() && !matches!(self, Self::NotChecked { .. })
    }
}

impl fmt::Display for GridCheckSkip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGrid => f.write_str("no grid exported"),
            Self::GridWithheld => f.write_str("grid withheld (needs review)"),
            Self::OddMeter { meter } => write!(f, "meter {meter}"),
            Self::XmlOnly => f.write_str("file not written (XML only)"),
            Self::NotAnalysed => f.write_str("the written file was not analysed"),
        }
    }
}

impl fmt::Display for GridCheck {
    /// The grid report's and `sc-cli process`'s wording: `pass (bar line +0.4 ms, BPM
    /// +0.0003)`, `off by +7.2 ms (...)`, `BPM differs by +0.0120 (found 128.012)`,
    /// `meter differs (found 3/4)`, `no grid found`, `not checked: <why>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let line = |period: &CheckPeriod| match period {
            CheckPeriod::Bar => "bar line",
            CheckPeriod::Beat => "beat line (beat 1 is the user's)",
        };
        match self {
            Self::Pass {
                offset_ms,
                bpm_diff,
                period,
            } => write!(
                f,
                "pass ({} {:+.1} ms, BPM {:+.4})",
                line(period),
                tidy(*offset_ms, 1),
                tidy(*bpm_diff, 4)
            ),
            Self::OffBy {
                offset_ms,
                bpm_diff,
                period,
            } => write!(
                f,
                "off by {:+.1} ms ({}, BPM {:+.4})",
                tidy(*offset_ms, 1),
                line(period),
                tidy(*bpm_diff, 4)
            ),
            Self::BpmDiffers { bpm_diff, found } => write!(
                f,
                "BPM differs by {:+.4} (found {:.3})",
                tidy(*bpm_diff, 4),
                found.0
            ),
            Self::MeterDiffers { found } => write!(f, "meter differs (found {found})"),
            Self::NoGridFound => f.write_str("no grid found in the written file"),
            Self::NotChecked { reason } => write!(f, "not checked: {reason}"),
        }
    }
}

/// `x` as zero when it rounds to zero at `decimals`, so such a difference reads `+0.0`, never
/// `-0.0`.
fn tidy(x: f64, decimals: i32) -> f64 {
    if x.abs() < 0.5 * 10_f64.powi(-decimals) {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests;
