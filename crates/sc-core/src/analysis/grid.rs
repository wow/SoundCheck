//! The grid of one track: its meter and grouping, the static lattice, its fitness and the
//! alternatives a one-key fix flips to.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::units::{Bpm, SampleIndex};

/// The note value one pulse of a [`Meter`] stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum BeatUnit {
    /// A quarter note: 4/4, 3/4.
    Quarter,
    /// An eighth note: 6/8, 9/8, 5/8, 7/8, 10/8.
    Eighth,
    /// A dotted quarter: compound meters counted in their main beats (6/8 in 2).
    DottedQuarter,
}

/// A bar's pulse count and how the pulses group into beats.
///
/// `grouping` lists the pulses per beat in order and sums to `beats_per_bar`: 4/4 is
/// `[1, 1, 1, 1]` at the quarter, 6/8 is `[3, 3]` at the eighth, 9/8 aksak is `[2, 2, 2, 3]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Meter {
    /// Pulses per bar at `unit`.
    pub beats_per_bar: u8,
    /// The note value of one pulse.
    pub unit: BeatUnit,
    /// Pulses per beat, in order; sums to `beats_per_bar`.
    pub grouping: Vec<u8>,
}

impl Meter {
    /// A meter from its grouping; `beats_per_bar` is the sum.
    ///
    /// # Panics
    /// When `grouping` is empty, contains a zero, or sums to more than 255.
    #[must_use]
    pub fn new(unit: BeatUnit, grouping: &[u8]) -> Self {
        assert!(!grouping.is_empty(), "a meter needs at least one beat");
        assert!(
            grouping.iter().all(|&g| g > 0),
            "a beat has at least one pulse"
        );
        let sum: u32 = grouping.iter().map(|&g| u32::from(g)).sum();
        let beats_per_bar = u8::try_from(sum).expect("a bar has at most 255 pulses");
        Self {
            beats_per_bar,
            unit,
            grouping: grouping.to_vec(),
        }
    }

    /// 4/4.
    #[must_use]
    pub fn four_four() -> Self {
        Self::new(BeatUnit::Quarter, &[1, 1, 1, 1])
    }

    /// 3/4.
    #[must_use]
    pub fn three_four() -> Self {
        Self::new(BeatUnit::Quarter, &[1, 1, 1])
    }

    /// 6/8 as two groups of three.
    #[must_use]
    pub fn six_eight() -> Self {
        Self::new(BeatUnit::Eighth, &[3, 3])
    }

    /// 9/8 aksak, 2+2+2+3 (the Turkish and Balkan "karşılama" grouping).
    #[must_use]
    pub fn nine_eight_aksak() -> Self {
        Self::new(BeatUnit::Eighth, &[2, 2, 2, 3])
    }

    /// 9/8 with the long group first, 3+2+2+2.
    #[must_use]
    pub fn nine_eight_long_first() -> Self {
        Self::new(BeatUnit::Eighth, &[3, 2, 2, 2])
    }

    /// 5/8 as 2+3.
    #[must_use]
    pub fn five_eight() -> Self {
        Self::new(BeatUnit::Eighth, &[2, 3])
    }

    /// 7/8 as 2+2+3.
    #[must_use]
    pub fn seven_eight() -> Self {
        Self::new(BeatUnit::Eighth, &[2, 2, 3])
    }

    /// 10/8 as 3+2+2+3.
    #[must_use]
    pub fn ten_eight() -> Self {
        Self::new(BeatUnit::Eighth, &[3, 2, 2, 3])
    }

    /// Every meter the estimator scores, most common first.
    #[must_use]
    pub fn templates() -> Vec<Self> {
        vec![
            Self::four_four(),
            Self::three_four(),
            Self::six_eight(),
            Self::nine_eight_aksak(),
            Self::nine_eight_long_first(),
            Self::five_eight(),
            Self::seven_eight(),
            Self::ten_eight(),
        ]
    }

    /// Number of beats (groups) per bar.
    #[must_use]
    pub fn beats(&self) -> usize {
        self.grouping.len()
    }

    /// Every beat has the same number of pulses (4/4, 3/4, 6/8), so no grouping needs showing.
    #[must_use]
    pub fn is_regular(&self) -> bool {
        self.grouping.windows(2).all(|w| w[0] == w[1])
    }
}

impl fmt::Display for Meter {
    /// `4/4`, `6/8 · 3+3`, `9/8 · 2+2+2+3`: the badge text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unit {
            BeatUnit::Quarter => write!(f, "{}/4", self.beats_per_bar)?,
            BeatUnit::Eighth => write!(f, "{}/8", self.beats_per_bar)?,
            BeatUnit::DottedQuarter => write!(f, "{}/8", u32::from(self.beats_per_bar) * 3)?,
        }
        if self.grouping.len() > 1 && self.grouping.iter().any(|&g| g > 1) {
            let groups: Vec<String> = self.grouping.iter().map(u8::to_string).collect();
            write!(f, " · {}", groups.join("+"))?;
        }
        Ok(())
    }
}

/// Whether one static tempo describes the whole track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// Residual P95 < 12 ms, max < 30 ms, local range <= 0.03 BPM, |drift| < 200 ppm.
    Static,
    /// Residual P95 < 25 ms and max < 50 ms; usable, worth a look.
    StaticWarn,
    /// The tempo moves; the audio is left untouched and the numbers are shown.
    Drifts,
}

/// Calibrated three-state confidence; never a raw score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// Every term at or above 0.8.
    Green,
    /// The weakest term between 0.5 and 0.8.
    Amber,
    /// The weakest term below 0.5.
    Red,
}

/// Why a grid is not green, shown as chips next to the confidence ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// Beats sit off the lattice.
    Residuals,
    /// Few beats land within 25 ms of the lattice.
    Coverage,
    /// Lattice slots without a detected beat.
    Recall,
    /// The other octave scores nearly as well.
    OctaveMargin,
    /// Another beat scores nearly as well for beat 1.
    DownbeatMargin,
    /// The runner-up meter scores nearly as well.
    MeterMargin,
    /// The file's BPM tag disagrees by more than 2 %.
    TagDisagrees,
    /// The chosen BPM lies outside the user's DJ-app range.
    OutsideRange,
    /// Too little audio for a confident fit.
    Short,
    /// The tempo drifts.
    Drifts,
    /// No kick-band energy; the anchor came from the broadband onset.
    NoKick,
    /// Typed by hand: the model found too few beats to fit.
    Manual,
}

/// One-flip alternatives kept as metadata so `x2`, `/2` and `1`-`n` never re-analyse.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Alternatives {
    /// Double tempo, when inside the plausible range.
    pub octave_up: Option<Bpm>,
    /// Half tempo, when inside the plausible range.
    pub octave_down: Option<Bpm>,
    /// Beat offsets that could also be beat 1, best first (never 0).
    pub downbeat_shift_beats: Vec<i8>,
}

/// A tempo change point for re-fitted grids (rekordbox XML carries these).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TempoSegment {
    /// First sample the segment's tempo applies to.
    pub from: SampleIndex,
    /// Tempo of the segment.
    pub bpm: Bpm,
}

/// The static grid of one track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Grid {
    /// First downbeat (bar 1, beat 1) at the native rate.
    pub anchor: SampleIndex,
    /// Tempo at `meter.unit`, carried to two decimals on export.
    pub bpm: Bpm,
    /// Pulse count and grouping.
    pub meter: Meter,
    /// The meter that scored second, offered first in the picker.
    pub meter_runner_up: Option<Meter>,
    /// Index of the detected beat that is bar 1 beat 1.
    pub first_downbeat_index: u32,
    /// Bars per phrase, 8 unless evidence says 4 or 16.
    pub phrase_len_bars: u8,
    /// Tempo change points; empty for a static grid.
    pub segments: Vec<TempoSegment>,
    /// 95th percentile of |beat - lattice| against kick onsets, in milliseconds.
    pub residual_p95_ms: f32,
    /// Largest |beat - lattice|, in milliseconds.
    pub residual_max_ms: f32,
    /// Range of the local tempo over 128-beat windows, in BPM.
    pub local_bpm_range: f32,
    /// Linear tempo drift over the track, in parts per million.
    pub drift_ppm: f32,
    /// Static, static with a warning, or drifting.
    pub verdict: Verdict,
    /// Calibrated three-state confidence.
    pub confidence: Confidence,
    /// Chips explaining anything below green.
    pub reasons: Vec<Reason>,
    /// One-flip alternatives.
    pub alternatives: Alternatives,
}

impl Grid {
    /// Samples per pulse at `sample_rate`: `60 * sr / bpm`.
    #[must_use]
    pub fn samples_per_beat(&self, sample_rate: u32) -> f64 {
        60.0 * f64::from(sample_rate) / self.bpm.0
    }
}

#[cfg(test)]
mod tests;
