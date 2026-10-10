//! Types that cross the desktop IPC boundary.
//!
//! Audio never crosses IPC. Only job events, small reports and byte-bounded frames do, so every
//! type here has a byte budget that tests enforce. `#[ts(export)]` writes the TypeScript bindings
//! into `src/lib/ipc/generated/` when `cargo test -p sc-core` runs.

mod recovery;
mod track;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use recovery::{PendingChange, RecoveredChange, RecoveryOutcome, RecoveryStatus};
pub use track::{
    FitChoice, GridFitHeader, Listen, MeterFrame, RowUpdate, TrackEvent, TrackOpened, Version,
};

use crate::analysis::{AnalysisRecord, AnalysisSettings, Confidence, Reason, Verdict};
use crate::export::SoundcheckRecord;
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
    /// The file holds Serato data (cue points, beat grid, auto gain, ...), whose positions are
    /// stored in the audio's own time: cutting the start would move them. Same as
    /// `!serato_tags.is_empty()`.
    pub serato: bool,
    /// The kinds of Serato data found, each once, in [`SeratoTag`] order.
    pub serato_tags: Vec<SeratoTag>,
    /// The file or one of its tags could not be read, so Serato data cannot be ruled out; an
    /// in-place cut treats the file as holding it.
    pub serato_unknown: bool,
    /// What SoundCheck recorded when it last exported the file (its `SOUNDCHECK` tag), when the
    /// tag is present and readable.
    pub soundcheck: Option<SoundcheckRecord>,
    /// Why a `SOUNDCHECK` tag that is present could not be read (a later format version, a
    /// damaged value): the file was processed before, but what was done is not known.
    pub soundcheck_unreadable: Option<String>,
}

/// A kind of Serato data in a file's tags: ID3 `GEOB` objects described `Serato <name>`
/// (WAV, AIFF and MP3) or Vorbis comment fields named `SERATO_<NAME>` (FLAC), after the layout
/// documented in Jan Holthuis' "serato-tags" notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum SeratoTag {
    /// Cue points, loops and flip markers (`Serato Markers_`, `Serato Markers2`,
    /// `SERATO_MARKERS_V2`).
    Markers,
    /// The beat grid (`Serato BeatGrid`, `SERATO_BEATGRID`).
    BeatGrid,
    /// Tempo, auto gain and gain from Serato's analysis (`Serato Autotags`,
    /// `SERATO_AUTOGAIN`): stale after a level change.
    Autotags,
    /// The waveform overview (`Serato Overview`, `SERATO_OVERVIEW`).
    Overview,
    /// The analysis version (`Serato Analysis`, `SERATO_ANALYSIS`).
    Analysis,
    /// MP3 decoder offsets (`Serato Offsets_`).
    Offsets,
    /// Anything else Serato stores (`Serato RelVolAd`, `Serato VidAssoc`, ...).
    Other,
}

impl SeratoTag {
    /// The kind of an ID3 `GEOB` description, when it is Serato's (`Serato ` and a name).
    #[must_use]
    pub fn from_geob_description(description: &str) -> Option<Self> {
        let name = strip_prefix_ignore_case(description, "Serato ")?;
        Some(match name {
            "Markers_" | "Markers2" => Self::Markers,
            "BeatGrid" => Self::BeatGrid,
            "Autotags" => Self::Autotags,
            "Overview" => Self::Overview,
            "Analysis" => Self::Analysis,
            "Offsets_" => Self::Offsets,
            _ => Self::Other,
        })
    }

    /// The kind of a Vorbis comment field name, when it is Serato's (`SERATO_` and a name, any
    /// case).
    #[must_use]
    pub fn from_vorbis_name(name: &str) -> Option<Self> {
        let rest = strip_prefix_ignore_case(name, "SERATO_")?.to_ascii_uppercase();
        Some(match rest.as_str() {
            "MARKERS" | "MARKERS_V2" | "MARKERS2" => Self::Markers,
            "BEATGRID" => Self::BeatGrid,
            "AUTOGAIN" | "AUTOTAGS" => Self::Autotags,
            "OVERVIEW" => Self::Overview,
            "ANALYSIS" => Self::Analysis,
            "OFFSETS" => Self::Offsets,
            _ => Self::Other,
        })
    }
}

/// `s` without `prefix` (compared ASCII case-insensitively), when it starts with it.
fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
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
    /// The grid is the user's edit of the analysed one.
    pub edited: bool,
    /// The user confirmed the grid by ear; it needs no review.
    pub confirmed: bool,
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
            edited: false,
            confirmed: false,
        }
    }
}

/// Every analysed row's plan after a settings change, and the revision they were decided with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Replan {
    /// Increases with every settings change in a session.
    pub revision: u32,
    /// The plans, by file.
    pub plans: Vec<RowPlan>,
}

/// One row as the session holds it, for a window that reloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    /// The file.
    pub entry: FileEntry,
    /// Its analysis, when finished.
    pub row: Option<Box<RowAnalysis>>,
    /// Its plan, when analysed.
    pub plan: Option<Plan>,
}

/// Everything the session holds, in the order the files were added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    /// The settings revision the plans were decided with.
    pub revision: u32,
    /// The rows.
    pub rows: Vec<SessionRow>,
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

/// Analyse these files with these settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeRequest {
    /// Files by id, in the order to analyse them.
    pub file_ids: Vec<u32>,
    /// Analysis settings (part of the cache key).
    pub analysis: AnalysisSettings,
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
        /// The settings revision `plan` was decided with; a newer replan wins.
        revision: u32,
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
    /// See [`crate::Error::NotDjSafe`].
    NotDjSafe,
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
    /// See [`crate::Error::RekordboxUsbExport`].
    RekordboxUsbExport,
    /// See [`crate::Error::InPlaceRefused`].
    InPlaceRefused,
    /// See [`crate::Error::NoSpace`].
    NoSpace,
    /// See [`crate::Error::VerifyFailed`].
    VerifyFailed,
    /// See [`crate::Error::FileChanged`].
    FileChanged,
    /// See [`crate::Error::NothingToUndo`].
    NothingToUndo,
    /// See [`crate::Error::AlreadyExists`].
    AlreadyExists,
    /// See [`crate::Error::ListedTwice`].
    ListedTwice,
    /// See [`crate::Error::SameOutputName`].
    SameOutputName,
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
mod tests;
