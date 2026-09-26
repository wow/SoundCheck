//! Types that cross the desktop IPC boundary.
//!
//! Audio never crosses IPC. Only job events, small reports and byte-bounded frames do, so every
//! type here has a byte budget that tests enforce. `#[ts(export)]` writes the TypeScript bindings
//! into `src/lib/ipc/generated/` when `cargo test -p sc-core` runs.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::analysis::{AnalysisRecord, Confidence, Reason, Verdict};
use crate::plan::{Codec, Plan};
use crate::units::{Bpm, DbTp, Lu, Lufs, Seconds};

/// Largest JSON encoding allowed for a progress event or meter frame.
pub const MAX_EVENT_BYTES: usize = 256;

/// Largest JSON encoding allowed for one analysed row with its plan.
pub const MAX_ROW_BYTES: usize = 2048;

/// A batch job, unique within a session.
pub type JobId = u32;

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

/// The grid as the table shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RowGrid {
    /// Tempo of the meter's beat, to 0.01.
    pub bpm: Bpm,
    /// The meter badge, e.g. `9/8 · 2+2+2+3`.
    pub meter: String,
    /// Whether the meter is plain 4/4 (the table shows no badge then).
    pub four_four: bool,
    /// The runner-up meter's badge.
    pub meter_runner_up: Option<String>,
    /// Three-state confidence.
    pub confidence: Confidence,
    /// Why the confidence is not green.
    pub reasons: Vec<Reason>,
    /// Static, static with elevated residuals, or drifting.
    pub verdict: Verdict,
    /// Double tempo, when plausible.
    pub octave_up: Option<Bpm>,
    /// Half tempo, when plausible.
    pub octave_down: Option<Bpm>,
    /// Bar 1 from the start of the decoded audio.
    pub bar1: Seconds,
    /// 95th percentile of the residuals against kick onsets, ms.
    pub residual_p95_ms: f32,
    /// Largest residual, ms.
    pub residual_max_ms: f32,
    /// Tempo drift over the track, parts per million.
    pub drift_ppm: f32,
}

/// An analysis as the table uses it: no timeline, beats or activations cross IPC per row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RowAnalysis {
    /// Playing time after encoder delay and padding.
    pub duration: Seconds,
    /// Sample rate of the decoded audio, Hz.
    pub sample_rate: u32,
    /// Channels of the decoded audio.
    pub channels: u16,
    /// Integrated loudness.
    pub integrated: Option<Lufs>,
    /// S-P95, the DJ statistic.
    pub short_term_p95: Option<Lufs>,
    /// Loudness range.
    pub lra: Option<Lu>,
    /// True peak.
    pub true_peak: DbTp,
    /// The grid, when found.
    pub grid: Option<RowGrid>,
    /// Why there is no grid.
    pub grid_skipped: Option<String>,
    /// The file's BPM tag.
    pub tag_bpm: Option<Bpm>,
    /// Served from the analysis cache.
    pub cached: bool,
}

impl RowAnalysis {
    /// The row for `record`.
    #[must_use]
    pub fn from_record(record: &AnalysisRecord, cached: bool) -> Self {
        let l = &record.loudness;
        let sample_rate = record.spec.sample_rate;
        Self {
            duration: record.duration,
            sample_rate,
            channels: record.spec.channels,
            integrated: l.integrated,
            short_term_p95: l.short_term_p95,
            lra: l.lra,
            true_peak: l.true_peak,
            grid: record.grid.as_ref().map(|g| RowGrid {
                bpm: g.bpm,
                meter: g.meter.to_string(),
                four_four: g.meter == crate::Meter::four_four(),
                meter_runner_up: g.meter_runner_up.as_ref().map(ToString::to_string),
                confidence: g.confidence,
                reasons: g.reasons.clone(),
                verdict: g.verdict,
                octave_up: g.alternatives.octave_up,
                octave_down: g.alternatives.octave_down,
                bar1: g.anchor.to_seconds(sample_rate),
                residual_p95_ms: g.residual_p95_ms,
                residual_max_ms: g.residual_max_ms,
                drift_ppm: g.drift_ppm,
            }),
            grid_skipped: record.grid_skipped.clone(),
            tag_bpm: record.tags.bpm,
            cached,
        }
    }
}

/// A row's plan, sent when the settings change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RowPlan {
    /// The file.
    pub file_id: u32,
    /// What processing would do.
    pub plan: Plan,
}

/// What a batch job reports to the UI, in order: per file `started`, `progress` at most every
/// 100 ms, then exactly one of `analysed`, `failed` or `cancelled`; a `batch` summary at most
/// every 500 ms; `finished` last. A job that cannot start sends `aborted` and then `finished`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobEvent {
    /// A worker took the file.
    Started {
        /// The job.
        job_id: JobId,
        /// The file.
        file_id: u32,
    },
    /// Analysis progress of one file, `0.0..=1.0`.
    Progress {
        /// The job.
        job_id: JobId,
        /// The file.
        file_id: u32,
        /// Fraction done.
        fraction: f32,
    },
    /// The file's analysis and its plan under the current settings.
    Analysed {
        /// The job.
        job_id: JobId,
        /// The file.
        file_id: u32,
        /// The table's fields (boxed: much larger than every other event).
        row: Box<RowAnalysis>,
        /// What processing would do.
        plan: Plan,
    },
    /// The file could not be analysed; the job goes on.
    Failed {
        /// The job.
        job_id: JobId,
        /// The file.
        file_id: u32,
        /// Why.
        error: IpcError,
    },
    /// The job was cancelled before or while this file ran.
    Cancelled {
        /// The job.
        job_id: JobId,
        /// The file.
        file_id: u32,
    },
    /// Where the whole job stands.
    Batch {
        /// The job.
        job_id: JobId,
        /// Files with a final event.
        done: u32,
        /// Files in the job.
        total: u32,
        /// Remaining time, ms, once a file has finished.
        eta_ms: Option<u32>,
        /// Seconds of audio per second of wall time so far.
        realtime_x: Option<f32>,
    },
    /// The job could not start (for example, the beat model is missing); no file was touched.
    Aborted {
        /// The job.
        job_id: JobId,
        /// Why.
        error: IpcError,
    },
    /// The job is over.
    Finished {
        /// The job.
        job_id: JobId,
        /// Whether it was cancelled.
        cancelled: bool,
    },
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
        let ev = JobEvent::Progress {
            job_id: u32::MAX,
            file_id: u32::MAX,
            fraction: 0.333_333_34,
        };
        assert!(json_len(&ev) <= MAX_EVENT_BYTES, "{} bytes", json_len(&ev));
        let ev = JobEvent::Batch {
            job_id: u32::MAX,
            done: u32::MAX,
            total: u32::MAX,
            eta_ms: Some(u32::MAX),
            realtime_x: Some(123.456_79),
        };
        assert!(json_len(&ev) <= MAX_EVENT_BYTES, "{} bytes", json_len(&ev));
    }

    #[test]
    fn a_worst_case_analysed_row_stays_within_budget() {
        use crate::analysis::Meter;
        use crate::plan::{GainPlan, ReviewReason};
        let grid = RowGrid {
            bpm: Bpm(123.456_789),
            meter: Meter::ten_eight().to_string(),
            four_four: false,
            meter_runner_up: Some(Meter::nine_eight_long_first().to_string()),
            confidence: Confidence::Red,
            reasons: vec![
                Reason::Residuals,
                Reason::Coverage,
                Reason::Recall,
                Reason::OctaveMargin,
                Reason::DownbeatMargin,
                Reason::MeterMargin,
                Reason::TagDisagrees,
                Reason::OutsideRange,
                Reason::Short,
                Reason::Drifts,
                Reason::NoKick,
            ],
            verdict: Verdict::Drifts,
            octave_up: Some(Bpm(246.913_578)),
            octave_down: Some(Bpm(61.728_394)),
            bar1: Seconds(1.234_567_89),
            residual_p95_ms: 12.345_678,
            residual_max_ms: 45.678_9,
            drift_ppm: -1_083.697_9,
        };
        let row = RowAnalysis {
            duration: Seconds(612.345_678_9),
            sample_rate: 192_000,
            channels: 2,
            integrated: Some(Lufs(-12.345_678_9)),
            short_term_p95: Some(Lufs(-10.123_456_7)),
            lra: Some(Lu(12.345_678_9)),
            true_peak: DbTp(-0.123_456_789),
            grid: Some(grid),
            grid_skipped: None,
            tag_bpm: Some(Bpm(123.45)),
            cached: false,
        };
        let plan = Plan {
            measured: Some(Lufs(-10.123_456_7)),
            gain: Some(GainPlan::GlobalGain {
                steps: -3,
                gain_db: -4.515_449_934_959_718,
                residual_lu: 0.392_006_765_040_282,
                true_peak_after: DbTp(-4.638_906_723_959_718),
            }),
            skip: None,
            review: vec![
                ReviewReason::Confidence,
                ReviewReason::Drifts,
                ReviewReason::OutsideBpmRange,
                ReviewReason::TagBpmDisagrees { tag: Bpm(123.45) },
            ],
            status: JobStage::NeedsReview,
        };
        let ev = JobEvent::Analysed {
            job_id: u32::MAX,
            file_id: u32::MAX,
            row: Box::new(row),
            plan,
        };
        assert!(json_len(&ev) <= MAX_ROW_BYTES, "{} bytes", json_len(&ev));
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
