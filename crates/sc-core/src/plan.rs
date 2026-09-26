//! What processing would do to a file: the loudness settings a batch is decided against, and the
//! plan for one file (gain, skip, what a human should look at). Deciding is a pure function of an
//! analysis record and these settings, so switching DJ | Streaming or editing the target replans
//! every row without re-analysis.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ipc::JobStage;
use crate::units::{Bpm, DbTp, Lufs};

/// The level change of one MPEG `global_gain` step: 2^(1/4) in amplitude, 1.5051 dB.
pub const GLOBAL_GAIN_STEP_DB: f64 = 1.505_149_978_319_906;

/// Gains and shortfalls below this are treated as zero (the table shows one decimal).
pub const NEGLIGIBLE_DB: f64 = 0.05;

/// A file's BPM tag disagrees with the analysis when they differ by more than this fraction.
pub const TAG_BPM_TOLERANCE: f64 = 0.02;

/// The audio codecs SoundCheck recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Codec {
    /// PCM in RIFF/RF64 WAVE.
    Wav,
    /// PCM in AIFF or AIFF-C.
    Aiff,
    /// FLAC.
    Flac,
    /// MPEG-1/2 Layer III.
    Mp3,
    /// AAC in MP4/M4A or ADTS.
    Aac,
    /// Apple Lossless in MP4/M4A.
    Alac,
    /// Ogg Vorbis.
    Vorbis,
    /// Ogg Opus.
    Opus,
    /// Anything else symphonia could open.
    #[default]
    Other,
}

impl Codec {
    /// The codec a file name suggests; `.m4a`/`.mp4` read as AAC until the stream says ALAC.
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("wav" | "wave") => Self::Wav,
            Some("aif" | "aiff" | "aifc") => Self::Aiff,
            Some("flac") => Self::Flac,
            Some("mp3") => Self::Mp3,
            Some("m4a" | "mp4" | "aac") => Self::Aac,
            Some("alac") => Self::Alac,
            Some("ogg" | "oga") => Self::Vorbis,
            Some("opus") => Self::Opus,
            _ => Self::Other,
        }
    }

    /// Whether v0.1 can write this codec; the others are analysed only.
    #[must_use]
    pub fn is_writable(self) -> bool {
        matches!(self, Self::Wav | Self::Aiff | Self::Flac | Self::Mp3)
    }

    /// The short label the table shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Aiff => "AIFF",
            Self::Flac => "FLAC",
            Self::Mp3 => "MP3",
            Self::Aac => "AAC",
            Self::Alac => "ALAC",
            Self::Vorbis => "OGG",
            Self::Opus => "OPUS",
            Self::Other => "?",
        }
    }
}

/// Which loudness statistic a batch aligns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum LoudnessMode {
    /// S-P95: the 95th percentile of the 3 s short-term loudness, for DJ sets.
    Dj,
    /// Integrated loudness (BS.1770-5), for streaming platforms.
    Streaming,
}

/// The settings every row's plan is decided against.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DecideSettings {
    /// Statistic to align.
    pub mode: LoudnessMode,
    /// Where the statistic should land.
    pub target: Lufs,
    /// True-peak ceiling a boost may not cross.
    pub ceiling: DbTp,
    /// The DJ app's BPM range; a grid outside it needs review.
    pub bpm_range: (Bpm, Bpm),
}

impl DecideSettings {
    /// The DJ default: S-P95 at -11 LUFS, ceiling -0.5 dBTP, BPM range 70-180.
    #[must_use]
    pub fn dj() -> Self {
        Self {
            mode: LoudnessMode::Dj,
            target: Lufs(-11.0),
            ceiling: DbTp(-0.5),
            bpm_range: (Bpm(70.0), Bpm(180.0)),
        }
    }

    /// The streaming default: integrated at -14 LUFS, ceiling -1.0 dBTP, BPM range 70-180.
    #[must_use]
    pub fn streaming() -> Self {
        Self {
            mode: LoudnessMode::Streaming,
            target: Lufs(-14.0),
            ceiling: DbTp(-1.0),
            ..Self::dj()
        }
    }
}

/// The level change processing would make.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GainPlan {
    /// Linear gain on decoded audio (WAV, AIFF, FLAC).
    Gain {
        /// The gain, dB; negative turns down.
        gain_db: f64,
        /// How far the ceiling kept a boost from the target, LU (0 when it did not).
        short_by_lu: f64,
        /// True peak after the gain.
        true_peak_after: DbTp,
    },
    /// Whole MPEG `global_gain` steps of [`GLOBAL_GAIN_STEP_DB`]; the rest stays as residual.
    GlobalGain {
        /// Steps, negative turns down.
        steps: i32,
        /// `steps * GLOBAL_GAIN_STEP_DB`.
        gain_db: f64,
        /// Target minus where the steps land, LU.
        residual_lu: f64,
        /// True peak after the gain.
        true_peak_after: DbTp,
    },
    /// Within [`NEGLIGIBLE_DB`] of the target already.
    AtTarget,
}

/// Why a file will not be processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SkipReason {
    /// The codec is analysed but not written in this version.
    AnalyseOnly {
        /// The codec.
        codec: Codec,
    },
    /// No audio above the -70 LUFS gate, so there is no loudness to align.
    Silent,
}

/// What a human should look at before processing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReviewReason {
    /// The grid's confidence is amber or red; the grid's own reasons say why.
    Confidence,
    /// The grid fits, but its residuals are elevated.
    CheckGrid,
    /// The tempo moves; a static grid will not fit everywhere.
    Drifts,
    /// The BPM lies outside the DJ app's range.
    OutsideBpmRange,
    /// The file's BPM tag differs by more than [`TAG_BPM_TOLERANCE`].
    TagBpmDisagrees {
        /// The tagged BPM.
        tag: Bpm,
    },
    /// A grid was requested but no beats were found.
    NoGrid,
}

/// What processing would do to one file, and where the row stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// The statistic the settings align on, as measured; `None` for silence.
    pub measured: Option<Lufs>,
    /// The level change; `None` when the file is skipped.
    pub gain: Option<GainPlan>,
    /// Why the file will not be processed.
    pub skip: Option<SkipReason>,
    /// What to look at first; empty when nothing needs a look.
    pub review: Vec<ReviewReason>,
    /// `Skipped`, `NeedsReview` or `Analysed`.
    pub status: JobStage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_gain_step_is_a_quarter_power_of_two() {
        let step = 20.0 * 2f64.powf(0.25).log10();
        assert!((step - GLOBAL_GAIN_STEP_DB).abs() < 1e-12, "{step}");
    }

    #[test]
    fn codecs_from_names() {
        assert_eq!(Codec::from_path(Path::new("a/B.FLAC")), Codec::Flac);
        assert_eq!(Codec::from_path(Path::new("x.aif")), Codec::Aiff);
        assert_eq!(Codec::from_path(Path::new("x.m4a")), Codec::Aac);
        assert_eq!(Codec::from_path(Path::new("x")), Codec::Other);
        assert!(Codec::Mp3.is_writable());
        assert!(!Codec::Alac.is_writable());
    }

    #[test]
    fn plans_serialise_as_tagged_unions() {
        let gain = GainPlan::Gain {
            gain_db: -3.2,
            short_by_lu: 0.0,
            true_peak_after: DbTp(-3.5),
        };
        assert_eq!(
            serde_json::to_string(&gain).unwrap(),
            r#"{"type":"gain","gainDb":-3.2,"shortByLu":0.0,"truePeakAfter":-3.5}"#
        );
        assert_eq!(
            serde_json::to_string(&SkipReason::AnalyseOnly { codec: Codec::Alac }).unwrap(),
            r#"{"type":"analyseOnly","codec":"alac"}"#
        );
    }
}
