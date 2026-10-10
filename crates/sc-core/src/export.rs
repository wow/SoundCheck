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

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::plan::Codec;
use crate::render::{BextLoudness, Tag};
use crate::units::{Lufs, SampleIndex, Seconds};

mod record;

pub use record::{
    MAX_RECORD_BYTES, RecordError, RecordGain, SOUNDCHECK_RECORD_VERSION, SoundcheckRecord,
};

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
    /// Prepare: the start is cut so the first bar line after the lead lands at the lead.
    Cut {
        /// Frames removed from the start (at least 1).
        #[ts(type = "number")]
        frames: u64,
        /// `frames` in seconds.
        seconds: Seconds,
    },
    /// Prepare, and the file already starts on a bar line: one lies before the lead (bar 1, or
    /// a line extrapolated from it by whole bars), so nothing is cut.
    OnBar {
        /// That bar line.
        bar_line: SampleIndex,
        /// `bar_line` in seconds.
        bar_line_s: Seconds,
        /// Bar 1 as shown (the grid's anchor); `bar_line` is bar 1 itself when they are equal.
        bar1: SampleIndex,
        /// `bar1` in seconds.
        bar1_s: Seconds,
    },
    /// Prepare, but the first bar line lies more than one beat after the lead: cutting to it
    /// would remove music, so nothing is cut and the grid travels in tags and the XML.
    NotCut {
        /// Bar 1 as shown (the grid's anchor).
        bar1: SampleIndex,
        /// `bar1` in seconds.
        bar1_s: Seconds,
        /// The bar line nearest the start (bar 1, or a line extrapolated from it by whole bars).
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

/// Something about a written file the user should know.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportNotice {
    /// A cut copy of a file with Serato data: its Serato cue points and beat grid are carried
    /// byte for byte, so in Serato they sit `cut_s` late in the copy (the original keeps them).
    SeratoCuesShifted {
        /// The cut, seconds.
        cut_s: Seconds,
    },
}

/// What exporting writes into one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    /// Gain applied to every sample, dB; 0 for grid only (the samples stay bit for bit).
    pub gain_db: f64,
    /// Frames to cut from the start, as asked of the renderer, which makes the cut up to 1 ms
    /// earlier at the quietest frame and reports the cut it made.
    #[ts(type = "number")]
    pub trim_frames: u64,
    /// Frames the output must have: the source's minus the trim (Library: the source's). A cut
    /// the renderer makes earlier leaves as many frames more as it moved.
    #[ts(type = "number")]
    pub expect_frames: u64,
    /// Output bits per sample, 16 or 24; `None` keeps the source depth.
    pub bits: Option<u8>,
    /// Tag items by neutral name, written only into a tag the file already has.
    pub tags: Vec<Tag>,
    /// Loudness for an existing Broadcast Wave `bext` chunk (WAV only), after the gain. Its
    /// fields keep `snake_case` keys in JSON (`integrated_lufs_x100`, ...), as render requests persist
    /// them, unlike the `camelCase` fields around it.
    pub bext: Option<BextLoudness>,
    /// What happens at the start of the file, and why.
    pub cut: Cut,
    /// What the user should know about the written file.
    pub notices: Vec<ExportNotice>,
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
    /// Grid only on a source the writer would not copy sample for sample: anything but 16- or
    /// 24-bit integer PCM (floating point, 8-bit, 20 bits in a 24-bit container, 32-bit) is
    /// written at another depth.
    GridOnlyWouldRequantise {
        /// The source is floating point.
        float: bool,
        /// The source's significant bits per sample, when known.
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
    /// Prepare in place would cut a file whose tags could not be read, so Serato data (whose
    /// cue points would move) cannot be ruled out. Export to a folder or use Library mode.
    SeratoUnknownInPlaceCut {
        /// The cut that was planned, seconds.
        cut_s: Seconds,
    },
    /// The sample rate is not one DJ players accept (44.1 or 48 kHz); converting it comes later.
    NotDjSafeRate {
        /// The file's sample rate, Hz.
        sample_rate_hz: u32,
    },
    /// More than two channels: DJ players and the writer take mono or stereo only.
    UnsupportedChannels {
        /// The file's channel count.
        channels: u16,
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
    /// Only the rekordbox XML could carry the file's grid, and the XML is off.
    NothingToWrite {
        /// Why the file itself is not written.
        reason: XmlOnlyReason,
    },
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

/// `x` with a negative zero made positive, so `-0.0` never prints as `-0.00`.
#[must_use]
pub fn positive_zero(x: f64) -> f64 {
    x + 0.0
}

#[cfg(test)]
mod tests;
