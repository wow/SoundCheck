//! The analysis record: what `sc-cli analyze` prints, what the cache stores and what the desktop
//! table binds to.
//!
//! Loudness terms follow ITU-R BS.1770-5 and EBU R 128 (Tech 3341 for M/S/I, Tech 3342 for
//! LRA). The grid model is a static lattice: one anchor sample, one tempo, one meter with its
//! grouping, plus the evidence needed to refit it without decoding again. Every position is a
//! [`SampleIndex`] at the file's native rate; seconds are derived on display.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::audio::AudioSpec;
use crate::units::{Bpm, DbFs, DbTp, Lu, Lufs, SampleIndex, Seconds};

/// Schema of [`AnalysisRecord`]; bumped only with a breaking change to the record.
pub const RECORD_SCHEMA: u32 = 1;

/// Hop of the short-term loudness timeline, in milliseconds (EBU Tech 3341 meters update at
/// >= 10 Hz).
pub const TIMELINE_HOP_MS: u32 = 100;

/// Loudness statistics of one track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessReport {
    /// Integrated loudness (BS.1770-5 gated); `None` when every block was gated (silence).
    pub integrated: Option<Lufs>,
    /// Loudest 400 ms momentary window; `None` for silence.
    pub momentary_max: Option<Lufs>,
    /// Loudest 3 s short-term window; `None` for silence.
    pub short_term_max: Option<Lufs>,
    /// 95th percentile of the short-term series above the -70 LUFS absolute gate (the DJ
    /// alignment statistic); `None` when no window passes the gate.
    pub short_term_p95: Option<Lufs>,
    /// Power mean of the loudest 30 s of short-term windows; `None` for tracks under 30 s.
    pub short_term_top30: Option<Lufs>,
    /// Loudness range (Tech 3342); `None` when fewer than two windows pass the gates.
    pub lra: Option<Lu>,
    /// True peak, 4x oversampled, or the sample peak when that is higher.
    pub true_peak: DbTp,
    /// Highest absolute sample value.
    pub sample_peak: DbFs,
    /// Peak-to-loudness ratio `true_peak - integrated`; `None` for silence.
    pub plr: Option<Lu>,
    /// The file is mono and was measured as dual mono (+3.01 LU against a single channel).
    pub dual_mono: bool,
    /// Short-term loudness over time for the waveform overlay.
    pub timeline: Timeline,
}

/// Short-term loudness sampled at a fixed hop.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    /// Hop between values, in milliseconds ([`TIMELINE_HOP_MS`]).
    pub hop_ms: u32,
    /// LUFS-S at each hop; `None` below the absolute gate.
    pub short_term: Vec<Option<f32>>,
}

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

/// Which beat-tracking model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Model {
    /// The bundled small model (about 10 MB, about 50x real time on an M1).
    #[default]
    Small,
    /// The full model, a developer option.
    Full,
}

/// Everything that changes an analysis result besides the audio itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSettings {
    /// The user's DJ-app BPM range; the octave is chosen inside it first.
    pub bpm_range: (Bpm, Bpm),
    /// Run beat tracking and the grid solver (false for loudness-only batches).
    pub grid: bool,
    /// Beat-tracking model.
    pub model: Model,
}

impl Default for AnalysisSettings {
    /// 70-180 BPM (rekordbox Normal mode), grid on, small model.
    fn default() -> Self {
        Self {
            bpm_range: (Bpm(70.0), Bpm(180.0)),
            grid: true,
            model: Model::Small,
        }
    }
}

impl AnalysisSettings {
    /// A stable 64-bit digest of the settings, part of the cache key. FNV-1a over a canonical
    /// text form, so the value does not depend on the Rust version or the platform.
    #[must_use]
    pub fn settings_hash(&self) -> u64 {
        let text = format!(
            "bpm_range={:.2}-{:.2};grid={};model={:?}",
            self.bpm_range.0.0, self.bpm_range.1.0, self.grid, self.model
        );
        fnv1a_64(text.as_bytes())
    }
}

/// FNV-1a, 64-bit (Fowler-Noll-Vo).
#[must_use]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes
        .iter()
        .fold(OFFSET, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(PRIME))
}

/// What the file's tags say, read once and never trusted over the audio.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TagHints {
    /// `TBPM` / `TXXX:BPM` / `bpm`, if present and numeric.
    pub bpm: Option<Bpm>,
    /// Genre tag, used only as an octave and meter prior.
    pub genre: Option<String>,
    /// Title for display.
    pub title: Option<String>,
    /// Artist for display.
    pub artist: Option<String>,
}

/// The cached inputs of the grid solver, enough to refit in under 5 ms without decoding.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GridEvidence {
    /// Model beats at the native rate.
    pub beats: Vec<SampleIndex>,
    /// Model downbeats at the native rate.
    pub downbeats: Vec<SampleIndex>,
    /// Downbeat activation per 20 ms frame (50 fps), for `1`-`n` alternatives.
    pub downbeat_logits_50fps: Vec<f32>,
    /// Kick-band onset peaks at 1 ms resolution, at the native rate.
    pub kick_onsets: Vec<SampleIndex>,
}

/// One track's complete analysis: the CLI's JSON, the cache entry and the table's row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRecord {
    /// [`RECORD_SCHEMA`].
    pub schema: u32,
    /// SoundCheck version that produced the record.
    pub version: String,
    /// Absolute path, NFC-normalised.
    pub path: String,
    /// File size in bytes, part of the cache key.
    #[ts(type = "number")]
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch, part of the cache key.
    #[ts(type = "number")]
    pub mtime_ns: i64,
    /// Sample rate and channels.
    pub spec: AudioSpec,
    /// Playable frames after encoder delay and padding.
    #[ts(type = "number")]
    pub frames: u64,
    /// `frames / sample_rate`.
    pub duration: Seconds,
    /// Encoder delay frames the decoder discarded (0 for lossless formats).
    pub delay: u32,
    /// Encoder padding frames the decoder discarded (0 for lossless formats).
    pub padding: u32,
    /// Loudness statistics.
    pub loudness: LoudnessReport,
    /// The grid, when requested and found.
    pub grid: Option<Grid>,
    /// Why there is no grid: `"not requested"`, `"shorter than 10 s"`, `"no beats found"`.
    pub grid_skipped: Option<String>,
    /// Tag hints.
    pub tags: TagHints,
    /// Solver inputs for refits; absent when the grid was not requested.
    pub evidence: Option<GridEvidence>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact values are intended in these tests
    use super::*;

    #[test]
    fn meter_templates_are_consistent_and_labelled() {
        for m in Meter::templates() {
            let sum: u32 = m.grouping.iter().map(|&g| u32::from(g)).sum();
            assert_eq!(u32::from(m.beats_per_bar), sum, "{m}");
        }
        assert_eq!(Meter::four_four().to_string(), "4/4");
        assert_eq!(Meter::three_four().to_string(), "3/4");
        assert_eq!(Meter::six_eight().to_string(), "6/8 · 3+3");
        assert_eq!(Meter::nine_eight_aksak().to_string(), "9/8 · 2+2+2+3");
        assert_eq!(Meter::seven_eight().to_string(), "7/8 · 2+2+3");
        assert!(Meter::four_four().is_regular());
        assert!(Meter::six_eight().is_regular());
        assert!(!Meter::nine_eight_aksak().is_regular());
        assert_eq!(Meter::nine_eight_aksak().beats(), 4);
    }

    #[test]
    fn settings_hash_is_stable_and_sensitive() {
        let a = AnalysisSettings::default();
        assert_eq!(a.settings_hash(), a.clone().settings_hash());
        assert_eq!(
            a.settings_hash(),
            0x4d9f_9a82_8cbc_1c2a,
            "canonical text changed"
        );
        let b = AnalysisSettings {
            grid: false,
            ..AnalysisSettings::default()
        };
        let c = AnalysisSettings {
            bpm_range: (Bpm(80.0), Bpm(180.0)),
            ..AnalysisSettings::default()
        };
        assert_ne!(a.settings_hash(), b.settings_hash());
        assert_ne!(a.settings_hash(), c.settings_hash());
    }

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn grid_geometry() {
        let grid = Grid {
            anchor: SampleIndex(22_050),
            bpm: Bpm(128.0),
            meter: Meter::four_four(),
            meter_runner_up: None,
            first_downbeat_index: 0,
            phrase_len_bars: 8,
            segments: vec![],
            residual_p95_ms: 3.0,
            residual_max_ms: 9.0,
            local_bpm_range: 0.01,
            drift_ppm: 4.0,
            verdict: Verdict::Static,
            confidence: Confidence::Green,
            reasons: vec![],
            alternatives: Alternatives::default(),
        };
        assert_eq!(grid.samples_per_beat(44_100), 20_671.875);
    }

    #[test]
    fn record_serialises_in_camel_case_with_units_flattened() {
        let record = AnalysisRecord {
            schema: RECORD_SCHEMA,
            version: crate::VERSION.into(),
            path: "/music/track.flac".into(),
            size: 1,
            mtime_ns: 2,
            spec: AudioSpec::CD,
            frames: 44_100,
            duration: Seconds(1.0),
            delay: 0,
            padding: 0,
            loudness: LoudnessReport {
                integrated: Some(Lufs(-11.0)),
                momentary_max: None,
                short_term_max: None,
                short_term_p95: Some(Lufs(-9.5)),
                short_term_top30: None,
                lra: Some(Lu(4.0)),
                true_peak: DbTp(-0.3),
                sample_peak: DbFs(-0.5),
                plr: Some(Lu(10.7)),
                dual_mono: false,
                timeline: Timeline {
                    hop_ms: TIMELINE_HOP_MS,
                    short_term: vec![None, Some(-12.0)],
                },
            },
            grid: None,
            grid_skipped: Some("not requested".into()),
            tags: TagHints::default(),
            evidence: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("\"shortTermP95\":-9.5"), "{json}");
        assert!(json.contains("\"shortTerm\":[null,-12.0]"), "{json}");
        assert!(json.contains("\"gridSkipped\":\"not requested\""), "{json}");
        let back: AnalysisRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, record);
    }
}
