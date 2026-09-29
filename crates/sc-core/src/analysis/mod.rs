//! The analysis record: what `sc-cli analyze` prints, what the cache stores and what the desktop
//! table binds to.
//!
//! Loudness terms follow ITU-R BS.1770-5 and EBU R 128 (Tech 3341 for M/S/I, Tech 3342 for
//! LRA). The grid model is a static lattice: one anchor sample, one tempo, one meter with its
//! grouping, plus the evidence needed to refit it without decoding again. Every position is a
//! [`SampleIndex`](crate::SampleIndex) at the file's native rate; seconds are derived on display.

mod edit;
mod grid;

pub use edit::{
    EDIT_BPM_LIMITS, EDIT_MAX_PULSES, EDIT_OCTAVE_LIMIT, GridEdit, GridEvidence, GridFit, OnsetList,
};
pub use grid::{Alternatives, BeatUnit, Confidence, Grid, Meter, Reason, TempoSegment, Verdict};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::audio::AudioSpec;
use crate::units::{Bpm, DbFs, DbTp, Lu, Lufs, Seconds};

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
mod tests;
