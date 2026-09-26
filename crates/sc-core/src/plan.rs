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

/// Targets a user may set, LUFS: from quiet broadcast levels to the loudest club masters.
pub const TARGET_RANGE_LUFS: (f64, f64) = (-30.0, -4.0);

/// Ceilings a user may set, dBTP. Never above full scale: gain only, and a boost stops at it.
pub const CEILING_RANGE_DBTP: (f64, f64) = (-6.0, 0.0);

/// BPM ranges a DJ app can be set to.
pub const BPM_LIMITS: (f64, f64) = (40.0, 300.0);

/// Checks a BPM range: both ends inside [`BPM_LIMITS`], low below high.
///
/// # Errors
/// [`crate::Error::InvalidArgument`] naming the problem.
pub fn check_bpm_range(range: (Bpm, Bpm)) -> crate::Result<()> {
    let (lo, hi) = (range.0.0, range.1.0);
    if !(lo.is_finite() && hi.is_finite() && lo >= BPM_LIMITS.0 && hi <= BPM_LIMITS.1 && lo < hi) {
        return Err(crate::Error::InvalidArgument(format!(
            "BPM range {lo}-{hi}: both ends must lie in {}-{} and the first below the second",
            BPM_LIMITS.0, BPM_LIMITS.1
        )));
    }
    Ok(())
}

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
    /// Checks the target, ceiling and BPM range against their limits.
    ///
    /// # Errors
    /// [`crate::Error::InvalidArgument`] naming the first value out of range.
    pub fn validate(&self) -> crate::Result<()> {
        let within = |v: f64, (lo, hi): (f64, f64)| v.is_finite() && v >= lo && v <= hi;
        if !within(self.target.0, TARGET_RANGE_LUFS) {
            return Err(crate::Error::InvalidArgument(format!(
                "target {} LUFS: must lie in {} to {} LUFS",
                self.target.0, TARGET_RANGE_LUFS.0, TARGET_RANGE_LUFS.1
            )));
        }
        if !within(self.ceiling.0, CEILING_RANGE_DBTP) {
            return Err(crate::Error::InvalidArgument(format!(
                "ceiling {} dBTP: must lie in {} to {} dBTP",
                self.ceiling.0, CEILING_RANGE_DBTP.0, CEILING_RANGE_DBTP.1
            )));
        }
        check_bpm_range(self.bpm_range)
    }

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
    fn settings_outside_their_limits_are_rejected() {
        assert!(DecideSettings::dj().validate().is_ok());
        assert!(DecideSettings::streaming().validate().is_ok());
        let above_full_scale = DecideSettings {
            ceiling: DbTp(0.5),
            ..DecideSettings::dj()
        };
        assert!(above_full_scale.validate().is_err());
        let too_loud = DecideSettings {
            target: Lufs(-2.0),
            ..DecideSettings::dj()
        };
        assert!(too_loud.validate().is_err());
        let nan = DecideSettings {
            target: Lufs(f64::NAN),
            ..DecideSettings::dj()
        };
        assert!(nan.validate().is_err());
        assert!(check_bpm_range((Bpm(180.0), Bpm(70.0))).is_err());
        assert!(check_bpm_range((Bpm(20.0), Bpm(70.0))).is_err());
        assert!(check_bpm_range((Bpm(80.0), Bpm(160.0))).is_ok());
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
