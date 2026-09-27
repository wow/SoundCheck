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
/// 2: [`GridEvidence`] holds the solver's exact inputs.
pub const RECORD_SCHEMA: u32 = 2;

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

/// Onsets at the analysis rate, as parallel arrays (compact in JSON).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct OnsetList {
    /// Rate `frames` count at: the analysis rate (22 050 Hz), not the file's.
    pub sample_rate: u32,
    /// Onset positions in frames at `sample_rate`.
    pub frames: Vec<u32>,
    /// Energy rise of each onset in dB (how sharp the attack is).
    pub rise_db: Vec<f32>,
    /// Envelope level just after each rise, in dBFS (how loud the attack is).
    pub level_db: Vec<f32>,
}

/// The cached inputs of the grid solver and the meter estimator, exactly as they consumed them,
/// so a refit (a user's edit, or none) reproduces the analysed grid without decoding again.
///
/// Positions here are not native-rate [`SampleIndex`] values: the model's times are kept as the
/// `f32` seconds it returned and onsets as frames at the analysis rate, because converting
/// either to the file's rate and back would not give the same numbers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GridEvidence {
    /// Model beat times in seconds, as the tracker returned them.
    pub beats_s: Vec<f32>,
    /// Model downbeat times in seconds, as the tracker returned them.
    pub downbeats_s: Vec<f32>,
    /// Downbeat activation (raw logits) per 20 ms frame (50 fps).
    pub downbeat_logits_50fps: Vec<f32>,
    /// Kick-band (30-150 Hz) onsets.
    pub kick_onsets: OnsetList,
    /// Broadband onsets, the anchor's fallback when the kick band is silent.
    pub broadband_onsets: OnsetList,
}

/// A user's corrections to an analysed grid, kept as overrides rather than as a finished grid
/// so they still apply when the file is analysed again. The default is no edit.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", default)]
pub struct GridEdit {
    /// The meter, replacing the estimated one.
    pub meter: Option<Meter>,
    /// A typed tempo at the meter's unit, kept exactly (never snapped).
    pub bpm: Option<Bpm>,
    /// A tapped tempo: the grid takes the lattice of the fitted tempo nearest to it.
    pub tempo_hint: Option<Bpm>,
    /// Octave steps from the chosen tempo: +1 doubles it, -1 halves it.
    pub octave: i8,
    /// A bar line placed by the user, at the native rate; any bar line may be given, bar 1 is
    /// the first one at or after the first beat.
    pub anchor: Option<SampleIndex>,
    /// Which beat of the bar the solver finds with the other overrides is beat 1 (0 = as
    /// solved), when no anchor is placed. It is relative: a later meter or tempo change moves
    /// the bar it counts from, so an editor that wants bar 1 to stay put places `anchor`.
    pub downbeat_shift: u8,
}

/// Tempos an edit may set or produce, in BPM at the meter's unit: wide enough for any pulse a
/// DJ counts, narrow enough that a grid never has more than about 17 lines per second.
pub const EDIT_BPM_LIMITS: (f64, f64) = (20.0, 1000.0);

/// The most octave steps an edit may take from the chosen tempo.
pub const EDIT_OCTAVE_LIMIT: i8 = 3;

/// The most pulses a bar of an edited meter may have.
pub const EDIT_MAX_PULSES: u8 = 32;

impl GridEdit {
    /// No override is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Checks the overrides against [`EDIT_BPM_LIMITS`], [`EDIT_OCTAVE_LIMIT`] and a meter of
    /// 1 to [`EDIT_MAX_PULSES`] pulses whose groups sum to the bar.
    ///
    /// # Errors
    /// [`crate::Error::InvalidArgument`] naming the first value out of its limits.
    pub fn validate(&self) -> crate::Result<()> {
        let invalid = |what: String| Err(crate::Error::InvalidArgument(what));
        let (lo, hi) = EDIT_BPM_LIMITS;
        for (name, bpm) in [("BPM", self.bpm), ("tapped BPM", self.tempo_hint)] {
            if let Some(Bpm(b)) = bpm
                && !(lo..=hi).contains(&b)
            {
                return invalid(format!("{name} {b} is outside {lo}-{hi}"));
            }
        }
        if self.octave.unsigned_abs() > EDIT_OCTAVE_LIMIT.unsigned_abs() {
            return invalid(format!(
                "{} octave steps (at most {EDIT_OCTAVE_LIMIT})",
                self.octave
            ));
        }
        if let Some(m) = &self.meter {
            let sum: u32 = m.grouping.iter().map(|&g| u32::from(g)).sum();
            if m.beats_per_bar == 0
                || m.beats_per_bar > EDIT_MAX_PULSES
                || m.grouping.contains(&0)
                || sum != u32::from(m.beats_per_bar)
            {
                return invalid(format!("meter {m:?}"));
            }
        }
        Ok(())
    }
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

    fn record_with(evidence: Option<GridEvidence>) -> AnalysisRecord {
        let json = r#"{"schema":2,"version":"0","path":"/a.wav","size":1,"mtimeNs":2,
            "spec":{"sampleRate":44100,"channels":2},"frames":1,"duration":1.0,"delay":0,
            "padding":0,"loudness":{"integrated":null,"momentaryMax":null,"shortTermMax":null,
            "shortTermP95":null,"shortTermTop30":null,"lra":null,"truePeak":-1.0,
            "samplePeak":-1.0,"plr":null,"dualMono":false,"timeline":{"hopMs":100,
            "shortTerm":[]}},"grid":null,"gridSkipped":null,"tags":{"bpm":null,"genre":null,
            "title":null,"artist":null},"evidence":null}"#;
        let mut record: AnalysisRecord = serde_json::from_str(json).unwrap();
        record.evidence = evidence;
        record
    }

    #[test]
    fn evidence_round_trips_exactly_through_json() {
        let onsets = OnsetList {
            sample_rate: 22_050,
            frames: vec![0, 11_025, u32::MAX],
            rise_db: vec![6.123_456_7, 30.0, f32::MIN_POSITIVE],
            level_db: vec![-80.0, -0.000_123_4, -12.345_678],
        };
        let evidence = GridEvidence {
            beats_s: vec![0.1, 0.52, 1.0 / 3.0, 123.456_79],
            downbeats_s: vec![0.52],
            downbeat_logits_50fps: vec![-3.256_789_4, 0.0, 2.123_456_7],
            kick_onsets: onsets.clone(),
            broadband_onsets: onsets,
        };
        let record = record_with(Some(evidence));
        let json = serde_json::to_string(&record).unwrap();
        let back: AnalysisRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.evidence, record.evidence, "every f32 bit survives");
    }

    #[test]
    fn grid_edits_are_checked_against_their_limits() {
        assert!(GridEdit::default().validate().is_ok());
        let ok = GridEdit {
            bpm: Some(Bpm(399.0)),
            octave: -3,
            meter: Some(Meter::ten_eight()),
            ..GridEdit::default()
        };
        assert!(ok.validate().is_ok());
        for bad in [
            GridEdit {
                bpm: Some(Bpm(f64::INFINITY)),
                ..GridEdit::default()
            },
            GridEdit {
                tempo_hint: Some(Bpm(1000.5)),
                ..GridEdit::default()
            },
            GridEdit {
                octave: 4,
                ..GridEdit::default()
            },
            GridEdit {
                meter: Some(Meter {
                    beats_per_bar: 0,
                    unit: BeatUnit::Quarter,
                    grouping: vec![],
                }),
                ..GridEdit::default()
            },
            GridEdit {
                meter: Some(Meter {
                    beats_per_bar: 4,
                    unit: BeatUnit::Quarter,
                    grouping: vec![1, 0, 3],
                }),
                ..GridEdit::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_grid_edit_defaults_to_no_edit_and_reads_missing_fields() {
        assert!(GridEdit::default().is_empty());
        let edit: GridEdit = serde_json::from_str(r#"{"octave":1}"#).unwrap();
        assert_eq!(edit.octave, 1);
        assert!(!edit.is_empty());
        assert_eq!(edit.anchor, None);
        let json = serde_json::to_string(&GridEdit {
            meter: Some(Meter::nine_eight_aksak()),
            anchor: Some(SampleIndex(44_100)),
            ..GridEdit::default()
        })
        .unwrap();
        assert!(json.contains("\"tempoHint\":null"), "{json}");
        assert!(json.contains("\"downbeatShift\":0"), "{json}");
    }
}
