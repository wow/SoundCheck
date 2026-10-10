//! What a lossless render is asked to do to one file: gain, head trim, output depth, the
//! loudness written into an existing Broadcast Wave `bext` chunk, and tag edits.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::units::{DbTp, Lu, Lufs};

/// The five loudness fields of a `bext` chunk, version 2 (EBU Tech 3285 v2, 2011, bytes
/// 412..422): each value x 100, rounded, as a little-endian signed 16-bit integer; a field that
/// was not measured holds [`BextLoudness::UNMEASURED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BextLoudness {
    /// `LoudnessValue`: integrated loudness, LUFS x 100.
    pub integrated_lufs_x100: i16,
    /// `LoudnessRange`: loudness range, LU x 100.
    pub range_lu_x100: i16,
    /// `MaxTruePeakLevel`: maximum true peak, dBTP x 100.
    pub max_true_peak_dbtp_x100: i16,
    /// `MaxMomentaryLoudness`: maximum momentary loudness, LUFS x 100.
    pub max_momentary_lufs_x100: i16,
    /// `MaxShortTermLoudness`: maximum short-term loudness, LUFS x 100.
    pub max_short_term_lufs_x100: i16,
}

impl BextLoudness {
    /// Tech 3285 v2: "If any of the loudness parameters are not being used then their 16-bit
    /// integer values shall be set to 7FFFh".
    pub const UNMEASURED: i16 = 0x7FFF;

    /// Every field "not measured": what a gain change without new measurements leaves, so the
    /// old values never go stale.
    pub const ALL_UNMEASURED: Self = Self {
        integrated_lufs_x100: Self::UNMEASURED,
        range_lu_x100: Self::UNMEASURED,
        max_true_peak_dbtp_x100: Self::UNMEASURED,
        max_momentary_lufs_x100: Self::UNMEASURED,
        max_short_term_lufs_x100: Self::UNMEASURED,
    };

    /// The fields from measurements; `None` and values that are not finite or do not fit
    /// (|value| >= 327.67) become [`Self::UNMEASURED`].
    #[must_use]
    pub fn from_measurements(
        integrated: Option<Lufs>,
        range: Option<Lu>,
        max_true_peak: Option<DbTp>,
        max_momentary: Option<Lufs>,
        max_short_term: Option<Lufs>,
    ) -> Self {
        Self {
            integrated_lufs_x100: field(integrated.map(|v| v.0)),
            range_lu_x100: field(range.map(|v| v.0)),
            max_true_peak_dbtp_x100: field(max_true_peak.map(|v| v.0)),
            max_momentary_lufs_x100: field(max_momentary.map(|v| v.0)),
            max_short_term_lufs_x100: field(max_short_term.map(|v| v.0)),
        }
    }

    /// The ten bytes as stored at offset 412 of the `bext` payload.
    #[must_use]
    pub fn to_le_bytes(&self) -> [u8; 10] {
        let fields = [
            self.integrated_lufs_x100,
            self.range_lu_x100,
            self.max_true_peak_dbtp_x100,
            self.max_momentary_lufs_x100,
            self.max_short_term_lufs_x100,
        ];
        let mut out = [0_u8; 10];
        for (i, v) in fields.iter().enumerate() {
            out[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// `round(100 x value)` as a field, or [`BextLoudness::UNMEASURED`].
fn field(value: Option<f64>) -> i16 {
    let Some(v) = value.map(|v| (v * 100.0).round()) else {
        return BextLoudness::UNMEASURED;
    };
    if v.is_finite() && v > f64::from(i16::MIN) && v < f64::from(BextLoudness::UNMEASURED) {
        // Checked to lie strictly inside the i16 range just above.
        #[allow(clippy::cast_possible_truncation)]
        let f = v as i16;
        f
    } else {
        BextLoudness::UNMEASURED
    }
}

/// A tag item to add or replace, by its container-neutral name: `BPM`, `INITIALKEY`, or any
/// other `NAME` of upper-case letters, digits and `_` (`REPLAYGAIN_TRACK_GAIN`, `SOUNDCHECK`).
/// The engine maps it to an ID3 frame or a Vorbis comment field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tag {
    /// The neutral name.
    pub name: String,
    /// The text value.
    pub value: String,
}

impl Tag {
    /// A tag item.
    #[must_use]
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            value: value.into(),
        }
    }

    /// Parses `NAME=VALUE` (split at the first `=`); the name is upper-cased.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] without `=` or a name.
    pub fn parse(text: &str) -> Result<Self> {
        let (name, value) = text
            .split_once('=')
            .ok_or_else(|| Error::InvalidArgument(format!("tag {text:?} is not NAME=VALUE")))?;
        if name.is_empty() {
            return Err(Error::InvalidArgument(format!("tag {text:?} has no name")));
        }
        Ok(Self {
            name: name.to_ascii_uppercase(),
            value: value.to_owned(),
        })
    }
}

/// A tag item SoundCheck adds or replaces: an ID3 frame label (`TBPM`, `TXXX:SOUNDCHECK`) or a
/// Vorbis comment field name, and its text value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TagEdit {
    /// Frame label or field name.
    pub label: String,
    /// Text value.
    pub value: String,
}

/// One render of a lossless file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RenderRequest {
    /// Gain applied to every sample, dB (finite; 0 keeps the samples bit for bit).
    pub gain_db: f64,
    /// Frames removed from the start (0 keeps the length). The render moves a cut up to 1 ms
    /// earlier to the quietest frame, unless [`Self::trim_snapped_from_frames`] says it already was.
    pub trim_frames: u64,
    /// Set when `trim_frames` is already the snapped cut of an earlier request (what
    /// `sc_io::render::snap_head_cut` gave for it): the cut that was requested, frames. The
    /// render then cuts exactly `trim_frames` (still fading it in) and reports this as the
    /// requested cut, so the output is the one a render of the original request makes. Snapping
    /// again could move the cut further back. Absent from requests that predate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_snapped_from_frames: Option<u64>,
    /// Output bits per sample, 16 or 24; `None` keeps the source depth (24 for a float source
    /// or one deeper than 24 bits, 16 for one of 16 bits or fewer).
    pub bits: Option<u8>,
    /// Loudness written into an existing `bext` chunk (one is never added).
    pub loudness: Option<BextLoudness>,
    /// Tag items to add or replace.
    pub tag_edits: Vec<TagEdit>,
}

#[cfg(test)]
mod tests;
