//! The SoundCheck engine: the one pipeline behind both the desktop app and `sc-cli`.
//!
//! [`analyze`] runs one file end to end (decode, loudness, beats, onsets, meter, grid, tags,
//! cache); [`batch`] runs many on worker threads with progress and cancellation; [`decide`]
//! says what processing would do to an analysed file. Nothing here
//! prints or knows about IPC; callers turn reports and events into text, JSON or IPC messages.

pub mod analyze;
pub mod batch;
pub mod cancel;
pub mod decide;
pub mod expand;

pub use analyze::{AnalyzeReport, Analyzer, CacheStatus, Progress, REPORT_SCHEMA, Timings};
pub use batch::{
    BatchFile, BatchProgress, BatchSettings, BatchSummary, EngineEvent, default_workers, run_batch,
};
pub use cancel::CancelToken;
pub use decide::decide;
pub use expand::{collect_audio_files, probe_all};
