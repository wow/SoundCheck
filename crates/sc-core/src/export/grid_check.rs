//! The self-check of an exported file's grid: the written file analysed afresh, its grid
//! compared with the grid the export recorded ([`super::ExportedGrid`]).
//!
//! The check is relative: the source's own analysis (the detector's grid, with the user's
//! meter, tempo octave and fitted part but none of their placed bar line, chosen beat 1 or
//! typed tempo) gave a bar line at some offset from the exported bar 1 and a tempo, recorded
//! with the export; the written file's analysis, solved with the same choices, is expected to
//! give a bar line within [`GRID_CHECK_OFFSET_MS`] of that same place (after a Prepare cut,
//! bar 1 sits at the lead), compared modulo the bar (bar 1 may be a whole bar later and still
//! be the same downbeat), and a tempo within [`GRID_CHECK_BPM_TOLERANCE`] of that tempo. So a
//! bar line the user nudged, a beat 1 they chose and a tempo they typed are neither confirmed
//! nor contradicted. For a grid the user did not edit, the source's analysis is the exported
//! grid, so the check compares with the exported grid itself.
//!
//! # What it proves
//!
//! It is the same detector reading the written audio again, and that detector is not stable
//! under a gain change or a sub-millisecond shift: gain alone can move the beat model's downbeat
//! votes, and with them its bar 1 by a whole beat, while no sample moved in time. The write is
//! refused unless the output has exactly the planned frame count, which pins how much was cut,
//! not where the music landed: it compares counts only. So:
//!
//! - agreement ([`GridCheck::Pass`]) says the detector still reads the grid where the export put
//!   it; it does not prove that another application's analysis agrees;
//! - disagreement does not show the file is wrong, and it cannot rule that out either. It says
//!   the detector reads this audio differently now. On a confirmed grid a real misplacement
//!   would show only as an advisory with its numbers.
//!
//! # The outcomes of a check that ran
//!
//! - A grid the user confirmed by ear: a disagreement of any kind is [`GridCheck::Advisory`],
//!   holding what was measured. The user's ear outranks the detector.
//! - A grid not confirmed, whose bar lines sit a whole number of beats from the expected place
//!   (a beat being a group of the meter: a quarter note in 4/4, three eighth notes in 6/8):
//!   [`GridCheck::CheckBar1`]. The beats still line up, so the question is which beat is bar 1,
//!   and that is what the detector is least sure of.
//! - A grid not confirmed, any other disagreement: [`GridCheck::OffBy`],
//!   [`GridCheck::BpmDiffers`], [`GridCheck::MeterDiffers`] or [`GridCheck::NoGridFound`], with
//!   the numbers. These are what [`GridCheck::failed`] reports; they are worth a look, and they
//!   never fail the export.
//!
//! The `result` value alone tells the outcomes apart. A reader that does not know a value treats
//! the check as absent (see [`super::ExportRecord::grid_check`]), so a later version can add one.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::analysis::Meter;
use crate::units::Bpm;

/// Largest difference between the fresh tempo and the exported one that counts as agreement,
/// BPM.
pub const GRID_CHECK_BPM_TOLERANCE: f64 = 0.005;

/// Largest distance of the fresh grid's nearest bar line from the exported bar 1 that counts as
/// agreement, milliseconds. Also how close to a whole number of beats an offset must be to read
/// as another beat ([`GridCheck::CheckBar1`]).
pub const GRID_CHECK_OFFSET_MS: f64 = 5.0;

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

/// What the written file's own analysis read differently from the exported grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GridDisagreement {
    /// The tempo agrees, but the nearest bar line is more than [`GRID_CHECK_OFFSET_MS`] away.
    OffBy {
        /// The fresh grid's bar line nearest the expected place minus that place,
        /// milliseconds.
        offset_ms: f64,
        /// The fresh tempo minus the expected one, BPM.
        bpm_diff: f64,
    },
    /// The tempo differs by more than [`GRID_CHECK_BPM_TOLERANCE`].
    BpmDiffers {
        /// The fresh tempo minus the expected one, BPM.
        bpm_diff: f64,
        /// The fresh tempo.
        found: Bpm,
    },
    /// The fresh grid has another meter, so its bar lines cannot be compared.
    MeterDiffers {
        /// The fresh meter.
        found: Meter,
        /// The exported meter; absent in results recorded before it was.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exported: Option<Meter>,
    },
    /// The written file's own analysis found no grid.
    NoGridFound,
}

/// The result of checking an exported file's grid against the written file's own analysis.
/// `result` alone says which outcome it is (see the module docs).
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
        /// The fresh grid's bar line nearest the expected place minus that place,
        /// milliseconds.
        offset_ms: f64,
        /// The fresh tempo minus the expected one, BPM.
        bpm_diff: f64,
    },
    /// The grid is the one the user confirmed, and the analysis reads the file differently. The
    /// confirmed grid was kept. A real misplacement would read the same way, so the numbers are
    /// here to look at.
    Advisory {
        /// What was measured.
        disagreement: GridDisagreement,
    },
    /// The grid was not confirmed, the tempo agrees, and the nearest bar line sits a whole number
    /// of the meter's beats (its groups: three eighth notes in 6/8) from the expected place, within
    /// [`GRID_CHECK_OFFSET_MS`]: the beats line up, but the analysis puts bar 1 on another beat,
    /// so the user should check which beat it is.
    CheckBar1 {
        /// The fresh grid's bar line nearest the expected place minus that place,
        /// milliseconds.
        offset_ms: f64,
        /// The fresh tempo minus the expected one, BPM.
        bpm_diff: f64,
        /// `offset_ms` in the meter's beats; never 0. Positive: the analysis puts bar 1 later.
        beats: i32,
    },
    /// The grid was not confirmed, the tempo agrees, but the nearest line is more than
    /// [`GRID_CHECK_OFFSET_MS`] away and not on another beat.
    OffBy {
        /// The fresh grid's bar line nearest the expected place minus that place,
        /// milliseconds.
        offset_ms: f64,
        /// The fresh tempo minus the expected one, BPM.
        bpm_diff: f64,
    },
    /// The grid was not confirmed, and the tempo differs by more than
    /// [`GRID_CHECK_BPM_TOLERANCE`].
    BpmDiffers {
        /// The fresh tempo minus the expected one, BPM.
        bpm_diff: f64,
        /// The fresh tempo.
        found: Bpm,
    },
    /// The grid was not confirmed, and the fresh grid has another meter, so its bar lines cannot
    /// be compared.
    MeterDiffers {
        /// The fresh meter.
        found: Meter,
        /// The exported meter; absent in results recorded before it was.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exported: Option<Meter>,
    },
    /// The grid was not confirmed, and the written file's own analysis found no grid.
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

    /// Whether the check ran on an unconfirmed grid and disagreed in a way another beat does not
    /// explain. Says nothing about the file being wrong (see the module docs), and never fails
    /// the export. False for [`Self::Advisory`] and [`Self::CheckBar1`], which are neither
    /// passes nor failures.
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(
            self,
            Self::OffBy { .. }
                | Self::BpmDiffers { .. }
                | Self::MeterDiffers { .. }
                | Self::NoGridFound
        )
    }

    /// This result on a grid the user confirmed: a disagreement of any kind becomes
    /// [`Self::Advisory`], with its numbers; agreement and "not checked" stay as they are.
    #[must_use]
    pub fn on_confirmed_grid(self) -> Self {
        let disagreement = match self {
            Self::OffBy {
                offset_ms,
                bpm_diff,
            }
            | Self::CheckBar1 {
                offset_ms,
                bpm_diff,
                ..
            } => GridDisagreement::OffBy {
                offset_ms,
                bpm_diff,
            },
            Self::BpmDiffers { bpm_diff, found } => {
                GridDisagreement::BpmDiffers { bpm_diff, found }
            }
            Self::MeterDiffers { found, exported } => {
                GridDisagreement::MeterDiffers { found, exported }
            }
            Self::NoGridFound => GridDisagreement::NoGridFound,
            other @ (Self::Pass { .. } | Self::Advisory { .. } | Self::NotChecked { .. }) => {
                return other;
            }
        };
        Self::Advisory { disagreement }
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

impl fmt::Display for GridDisagreement {
    /// `off by +7.2 ms (BPM +0.0003)`, `BPM differs by +0.0120 (found 128.012)`, `meter differs
    /// (found 3/4, exported 4/4)`, `no grid found in the written file`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OffBy {
                offset_ms,
                bpm_diff,
            } => write!(
                f,
                "off by {:+.1} ms (BPM {:+.4})",
                tidy(*offset_ms, 1),
                tidy(*bpm_diff, 4)
            ),
            Self::BpmDiffers { bpm_diff, found } => write!(
                f,
                "BPM differs by {:+.4} (found {:.3})",
                tidy(*bpm_diff, 4),
                found.0
            ),
            Self::MeterDiffers { found, exported } => meter_differs(f, found, exported.as_ref()),
            Self::NoGridFound => f.write_str("no grid found in the written file"),
        }
    }
}

impl fmt::Display for GridCheck {
    /// The grid report's and `sc-cli process`'s wording: `pass (bar line +0.4 ms, BPM +0.0003)`;
    /// `the analysis reads this differently; your confirmed grid was kept: off by -416.7 ms
    /// (BPM +0.0000)`; `check bar 1: the analysis reads it 1 beat earlier (bar line -416.7 ms, BPM
    /// +0.0000)`; `off by +7.2 ms (...)`, `BPM differs by +0.0120 (found 128.012)`, `meter differs
    /// (found 3/4, exported 4/4)`, `no grid found in the written file`; `not checked: <why>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass {
                offset_ms,
                bpm_diff,
            } => write!(
                f,
                "pass (bar line {:+.1} ms, BPM {:+.4})",
                tidy(*offset_ms, 1),
                tidy(*bpm_diff, 4)
            ),
            Self::Advisory { disagreement } => write!(
                f,
                "the analysis reads this differently; your confirmed grid was kept: \
                 {disagreement}"
            ),
            Self::CheckBar1 {
                offset_ms,
                bpm_diff,
                beats,
            } => write!(
                f,
                "check bar 1: the analysis reads it {} beat{} {} (bar line {:+.1} ms, BPM {:+.4})",
                beats.unsigned_abs(),
                if beats.unsigned_abs() == 1 { "" } else { "s" },
                if *beats < 0 { "earlier" } else { "later" },
                tidy(*offset_ms, 1),
                tidy(*bpm_diff, 4)
            ),
            Self::OffBy {
                offset_ms,
                bpm_diff,
            } => GridDisagreement::OffBy {
                offset_ms: *offset_ms,
                bpm_diff: *bpm_diff,
            }
            .fmt(f),
            Self::BpmDiffers { bpm_diff, found } => GridDisagreement::BpmDiffers {
                bpm_diff: *bpm_diff,
                found: *found,
            }
            .fmt(f),
            Self::MeterDiffers { found, exported } => meter_differs(f, found, exported.as_ref()),
            Self::NoGridFound => GridDisagreement::NoGridFound.fmt(f),
            Self::NotChecked { reason } => write!(f, "not checked: {reason}"),
        }
    }
}

/// `meter differs (found 3/4, exported 4/4)`, without the exported meter in a result recorded
/// before it was.
fn meter_differs(
    f: &mut fmt::Formatter<'_>,
    found: &Meter,
    exported: Option<&Meter>,
) -> fmt::Result {
    match exported {
        Some(exported) => write!(f, "meter differs (found {found}, exported {exported})"),
        None => write!(f, "meter differs (found {found})"),
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
