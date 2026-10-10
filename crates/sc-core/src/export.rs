//! What exporting a batch does to each file: the batch's export settings, the plan for one file
//! (gain, head trim, the frame count to expect, the tags and Broadcast Wave loudness written)
//! and, when the file is not written, why.
//!
//! Two batch modes. **Prepare** is for new tracks: a lossless file may lose at most one beat of
//! audio at its start, so the bar line before the music lands a short lead after the start of
//! the file (DJ apps then make bar 1 the first grid line). **Library** is for tracks already in a
//! DJ app, whose cue points are stored as positions: the length never changes.
//!
//! Tags use container-neutral names ([`Tag`]); Replay Gain 2.0 track values are relative to
//! -18 LUFS (Replay Gain 2.0 specification, which measures with ITU-R BS.1770).

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::plan::{Codec, LoudnessMode};
use crate::render::{BextLoudness, Tag};
use crate::units::{Bpm, Lufs, SampleIndex, Seconds};

/// The lead a Prepare cut leaves before bar 1, milliseconds, until it is calibrated against
/// DJ apps' own analysis.
pub const DEFAULT_LEAD_MS: f64 = 5.0;

/// Leads a user may set, milliseconds.
pub const LEAD_MS_RANGE: (f64, f64) = (0.0, 50.0);

/// Output depths an export may force, bits; `None` keeps the source's.
pub const EXPORT_DEPTHS: [u8; 2] = [16, 24];

/// Neutral tag name of the tempo (`TBPM` + `TXXX:BPM` in ID3, `BPM` in a Vorbis comment).
pub const TAG_BPM: &str = "BPM";
/// Neutral tag name of the Replay Gain 2.0 track gain.
pub const TAG_REPLAYGAIN_TRACK_GAIN: &str = "REPLAYGAIN_TRACK_GAIN";
/// Neutral tag name of the Replay Gain 2.0 track peak.
pub const TAG_REPLAYGAIN_TRACK_PEAK: &str = "REPLAYGAIN_TRACK_PEAK";
/// Neutral tag name of SoundCheck's own record of what it did ([`SoundcheckRecord`]).
pub const TAG_SOUNDCHECK: &str = "SOUNDCHECK";

/// The Replay Gain 2.0 reference level.
pub const REPLAYGAIN_REFERENCE: Lufs = Lufs(-18.0);

/// How a batch treats the length of its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum BatchMode {
    /// New tracks: lossless files may be cut at the start so bar 1 sits a lead after it.
    #[default]
    Prepare,
    /// Tracks already in a DJ app: the length never changes, so stored cue points stay put.
    Library,
}

impl BatchMode {
    /// `prepare` or `library`, as the SOUNDCHECK record writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Library => "library",
        }
    }
}

/// Where exported files go. The folder itself is the caller's to supply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Place {
    /// Replaces the original, after backing it up.
    #[default]
    InPlace,
    /// A copy in another folder; the original is untouched.
    Folder,
}

/// The export settings of a batch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    /// Prepare or Library.
    pub batch_mode: BatchMode,
    /// In place or into a folder.
    pub place: Place,
    /// Output bits per sample, one of [`EXPORT_DEPTHS`]; `None` keeps the source depth.
    pub depth: Option<u8>,
    /// Change no audio: no gain and no cut; only tags (and the XML) carry the grid.
    pub grid_only: bool,
    /// Write the tempo tag (`TBPM` and `TXXX:BPM` in ID3, `BPM` in FLAC).
    pub tbpm: bool,
    /// Write a rekordbox XML for the batch.
    pub xml: bool,
    /// Time a Prepare cut leaves before bar 1, milliseconds, in [`LEAD_MS_RANGE`].
    pub lead_ms: f64,
}

impl ExportSettings {
    /// The defaults of `batch_mode`: in place, source depth, gain on, the tempo tag in Prepare
    /// only (a Library track's tag is the DJ app's), XML on, [`DEFAULT_LEAD_MS`].
    #[must_use]
    pub fn new(batch_mode: BatchMode) -> Self {
        Self {
            batch_mode,
            place: Place::InPlace,
            depth: None,
            grid_only: false,
            tbpm: batch_mode == BatchMode::Prepare,
            xml: true,
            lead_ms: DEFAULT_LEAD_MS,
        }
    }

    /// Checks the depth and the lead against their limits.
    ///
    /// # Errors
    /// [`crate::Error::InvalidArgument`] naming the value out of range.
    pub fn validate(&self) -> crate::Result<()> {
        if let Some(d) = self.depth
            && !EXPORT_DEPTHS.contains(&d)
        {
            return Err(crate::Error::InvalidArgument(format!(
                "depth {d} bits: must be 16 or 24, or the source depth"
            )));
        }
        let (lo, hi) = LEAD_MS_RANGE;
        if !(self.lead_ms.is_finite() && self.lead_ms >= lo && self.lead_ms <= hi) {
            return Err(crate::Error::InvalidArgument(format!(
                "lead {} ms: must lie in {lo} to {hi} ms",
                self.lead_ms
            )));
        }
        Ok(())
    }
}

impl Default for ExportSettings {
    /// [`ExportSettings::new`] in Prepare mode.
    fn default() -> Self {
        Self::new(BatchMode::Prepare)
    }
}

/// What happens at the start of a file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Cut {
    /// Prepare: the start is cut so the bar line before the music lands at the lead (`frames`
    /// may be 0 when it already does).
    Cut {
        /// Frames removed from the start.
        #[ts(type = "number")]
        frames: u64,
        /// `frames` in seconds.
        seconds: Seconds,
    },
    /// Prepare, but bar 1 lies more than one beat after the lead: no music is removed, the grid
    /// travels in tags and the XML.
    NotCut {
        /// The first bar line in the file (extrapolated from bar 1 by whole bars).
        first_bar_line: SampleIndex,
        /// `first_bar_line` in seconds.
        first_bar_line_s: Seconds,
    },
    /// Prepare without a grid: nothing to cut to.
    NoGrid,
    /// Grid only: no audio changes.
    GridOnly,
    /// Library mode never changes the length.
    Library,
}

/// What exporting writes into one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    /// Gain applied to every sample, dB; 0 for grid only (the samples stay bit for bit).
    pub gain_db: f64,
    /// Frames removed from the start.
    #[ts(type = "number")]
    pub trim_frames: u64,
    /// Frames the output must have: the source's minus the trim (Library: the source's).
    #[ts(type = "number")]
    pub expect_frames: u64,
    /// Output bits per sample, 16 or 24; `None` keeps the source depth.
    pub bits: Option<u8>,
    /// Tag items by neutral name, written only into a tag the file already has.
    pub tags: Vec<Tag>,
    /// Loudness for an existing Broadcast Wave `bext` chunk (WAV only), after the gain.
    pub bext: Option<BextLoudness>,
    /// What happens at the start of the file, and why.
    pub cut: Cut,
}

/// Why only the batch's rekordbox XML carries a file's grid (the file is not written).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum XmlOnlyReason {
    /// MP3 and AAC files are not written yet (their lossless gain change comes later).
    Mp3OrAac {
        /// The codec.
        codec: Codec,
    },
    /// Grid only on FLAC: writing tags alone would re-encode the frames, which is not done yet.
    GridOnlyFlac,
    /// Grid only on a source the writer would requantise (floating point, or deeper than 24
    /// bits, becomes 24-bit integer), so the samples would change.
    GridOnlyWouldRequantise {
        /// The source is floating point.
        float: bool,
        /// The source's bits per sample, when known.
        bits: Option<u8>,
    },
    /// Grid only on a file without a tag: there is nothing to write (tags are not created).
    NoTagToWriteGridOnly,
}

/// Why a file is neither written nor listed for the XML.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportSkip {
    /// Prepare in place would cut a file holding Serato data, whose cue points are positions in
    /// the audio and would move. Export to a folder or use Library mode.
    SeratoInPlaceCut {
        /// The cut that was planned, seconds.
        cut_s: Seconds,
    },
    /// The sample rate is not one DJ players accept (44.1 or 48 kHz); converting it comes later.
    NotDjSafeRate {
        /// The file's sample rate, Hz.
        sample_rate_hz: u32,
    },
    /// The codec is analysed but never written in this version.
    Unsupported {
        /// The codec.
        codec: Codec,
    },
    /// No audio above the -70 LUFS gate: there is no loudness to align.
    Silent,
    /// Grid only on a file without a grid: there is nothing to write.
    NoGrid,
}

/// What exporting does with one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportOutcome {
    /// The file is written as planned.
    Write {
        /// The plan.
        plan: ExportPlan,
    },
    /// The file is left as it is; only the batch's rekordbox XML carries its grid.
    XmlOnly {
        /// Why.
        reason: XmlOnlyReason,
    },
    /// The file is left out.
    Skip {
        /// Why.
        reason: ExportSkip,
    },
}

/// The value of the `SOUNDCHECK` tag: what SoundCheck did to the file, as `key=value` pairs
/// joined by `;`, in this order:
/// `v=1;app=<version>;mode=prepare|library;stat=S-P95|I;target=<LUFS, 2 dp>;gain=<dB, signed,
/// 2 dp>;trim=<frames>;bpm=<2 dp>;bar1=<seconds, 3 dp>;src=<16 hex digits>`.
/// `bpm` and `bar1` are left out without a grid, `src` without a source hash.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundcheckRecord<'a> {
    /// The SoundCheck version.
    pub app: &'a str,
    /// The batch mode.
    pub mode: BatchMode,
    /// The statistic the gain aligned.
    pub stat: LoudnessMode,
    /// The target it aligned to.
    pub target: Lufs,
    /// The gain applied, dB.
    pub gain_db: f64,
    /// Frames removed from the start.
    pub trim_frames: u64,
    /// The grid's tempo at the meter's unit.
    pub bpm: Option<Bpm>,
    /// The first bar line of the exported audio.
    pub bar1: Option<Seconds>,
    /// BLAKE3 of the source file; its first 8 bytes are written.
    pub source_blake3: Option<[u8; 32]>,
}

/// The record's format version.
pub const SOUNDCHECK_RECORD_VERSION: u32 = 1;

impl SoundcheckRecord<'_> {
    /// The tag value (see the type's documentation).
    #[must_use]
    pub fn to_value(&self) -> String {
        let stat = match self.stat {
            LoudnessMode::Dj => "S-P95",
            LoudnessMode::Streaming => "I",
        };
        let mut v = format!(
            "v={SOUNDCHECK_RECORD_VERSION};app={};mode={};stat={stat};target={:.2};gain={:+.2};trim={}",
            self.app,
            self.mode.as_str(),
            positive_zero(self.target.0),
            positive_zero(self.gain_db),
            self.trim_frames
        );
        // Writing into a String cannot fail.
        if let Some(bpm) = self.bpm {
            let _ = write!(v, ";bpm={:.2}", bpm.0);
        }
        if let Some(bar1) = self.bar1 {
            let _ = write!(v, ";bar1={:.3}", positive_zero(bar1.0));
        }
        if let Some(hash) = self.source_blake3 {
            v.push_str(";src=");
            for b in &hash[..8] {
                let _ = write!(v, "{b:02x}");
            }
        }
        v
    }
}

/// `x` with a negative zero made positive, so `-0.0` never prints as `-0.00`.
#[must_use]
pub fn positive_zero(x: f64) -> f64 {
    x + 0.0
}

#[cfg(test)]
mod tests;
