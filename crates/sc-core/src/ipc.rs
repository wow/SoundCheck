//! Types that cross the desktop IPC boundary.
//!
//! Audio never crosses IPC. Only job events, small reports and byte-bounded frames do, so every
//! type here has a byte budget that tests enforce. `#[ts(export)]` writes the TypeScript bindings
//! into `src/lib/ipc/generated/` when `cargo test -p sc-core` runs.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::plan::Codec;
use crate::units::{DbTp, Lufs, Seconds};

/// Largest JSON encoding allowed for any single event or frame.
pub const MAX_EVENT_BYTES: usize = 256;

/// Where a file is in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    /// Waiting for a worker.
    Queued,
    /// Decoding, measuring and beat tracking.
    Analysing,
    /// Analysis finished; nothing written yet.
    Analysed,
    /// Analysis finished but a human should look before processing.
    NeedsReview,
    /// Rendering the output.
    Processing,
    /// Writing and verifying the output file.
    Writing,
    /// Written and verified.
    Done,
    /// Failed; the original is untouched.
    Failed,
    /// Cancelled by the user; partial outputs were removed.
    Cancelled,
    /// Skipped for a stated reason.
    Skipped,
}

/// Progress for one file inside a batch job, sent at most 10 times per second per file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    /// The batch job.
    pub job_id: u32,
    /// The file within the job.
    pub file_id: u32,
    /// Current stage.
    pub stage: JobStage,
    /// Completion of the current stage, `0.0..=1.0`.
    pub fraction: f32,
    /// Estimated remaining time for this file, if known.
    pub eta_ms: Option<u32>,
}

/// One meter reading for the previewed file, sent at most 30 times per second.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MeterFrame {
    /// The file being previewed.
    pub file_id: u32,
    /// Momentary loudness (400 ms window).
    pub momentary: Lufs,
    /// Short-term loudness (3 s window).
    pub short_term: Lufs,
    /// True peak since the last frame.
    pub true_peak: DbTp,
}

/// Why a file's format is not what DJ players handle everywhere; export would convert it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum DjUnsafe {
    /// Neither 44.1 nor 48 kHz.
    SampleRate,
    /// Neither 16- nor 24-bit.
    BitDepth,
    /// Floating-point samples.
    Float,
    /// Not stereo.
    Channels,
}

/// What a file is before it is decoded, read from its headers and tags.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    /// The codec; from the file name when the headers cannot be read.
    pub codec: Codec,
    /// Sample rate in Hz, when the headers say.
    pub sample_rate: Option<u32>,
    /// Channel count, when the headers say.
    pub channels: Option<u8>,
    /// Bits per sample of a PCM or lossless stream.
    pub bits_per_sample: Option<u8>,
    /// Floating-point samples (WAV format 3).
    pub float: bool,
    /// Audio bitrate in kbit/s, for lossy streams.
    pub bitrate_kbps: Option<u32>,
    /// Playing time from the headers (the decoder's frame count is authoritative).
    pub duration: Option<Seconds>,
    /// Title tag.
    pub title: Option<String>,
    /// Artist tag.
    pub artist: Option<String>,
    /// Album tag.
    pub album: Option<String>,
    /// The first way the format is not DJ-safe, if any.
    pub dj_unsafe: Option<DjUnsafe>,
}

/// A file the user added, with the id every later event uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// Session-wide id.
    pub file_id: u32,
    /// Absolute path, NFC-normalised.
    pub path: String,
    /// What the headers say.
    pub info: FileInfo,
}

/// Error classes as the UI sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum IpcErrorKind {
    /// See [`crate::Error::UnsupportedFormat`].
    UnsupportedFormat,
    /// See [`crate::Error::UnsupportedChannels`].
    UnsupportedChannels,
    /// See [`crate::Error::Corrupt`].
    Corrupt,
    /// See [`crate::Error::Io`].
    Io,
    /// See [`crate::Error::Cancelled`].
    Cancelled,
    /// See [`crate::Error::DrmProtected`].
    DrmProtected,
    /// See [`crate::Error::WouldClip`].
    WouldClip,
    /// See [`crate::Error::ModelUnavailable`].
    ModelUnavailable,
    /// See [`crate::Error::InvalidArgument`].
    InvalidArgument,
    /// Anything the engine did not classify.
    Internal,
}

/// An error crossing IPC: a class the UI can branch on plus a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    /// The class.
    pub kind: IpcErrorKind,
    /// Message for the user, already worded per the copy guidelines.
    pub message: String,
    /// The file concerned, if any.
    pub file_id: Option<u32>,
}

impl From<crate::Error> for IpcError {
    fn from(err: crate::Error) -> Self {
        Self {
            kind: err.kind(),
            message: err.to_string(),
            file_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_len<T: Serialize>(value: &T) -> usize {
        serde_json::to_vec(value).expect("serialisable").len()
    }

    #[test]
    fn progress_event_stays_within_budget() {
        let ev = ProgressEvent {
            job_id: u32::MAX,
            file_id: u32::MAX,
            stage: JobStage::NeedsReview,
            fraction: 0.333_333_34,
            eta_ms: Some(u32::MAX),
        };
        assert!(json_len(&ev) <= MAX_EVENT_BYTES, "{} bytes", json_len(&ev));
    }

    #[test]
    fn meter_frame_stays_within_budget() {
        let frame = MeterFrame {
            file_id: u32::MAX,
            momentary: Lufs(-123.456_789),
            short_term: Lufs(-123.456_789),
            true_peak: DbTp(-123.456_789),
        };
        assert!(
            json_len(&frame) <= MAX_EVENT_BYTES,
            "{} bytes",
            json_len(&frame)
        );
    }

    #[test]
    fn errors_map_to_their_class() {
        let ipc: IpcError = crate::Error::Cancelled.into();
        assert_eq!(ipc.kind, IpcErrorKind::Cancelled);
        assert_eq!(ipc.message, "cancelled");
        assert_eq!(
            serde_json::to_string(&JobStage::NeedsReview).unwrap(),
            "\"needsReview\""
        );
    }
}
