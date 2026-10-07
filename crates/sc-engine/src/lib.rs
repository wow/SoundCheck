//! The SoundCheck engine: the one pipeline behind both the desktop app and `sc-cli`.
//!
//! [`analyze`] runs one file end to end (decode, loudness, beats, onsets, meter, grid, tags,
//! cache); [`batch`] runs many on worker threads with progress and cancellation; [`edits`]
//! applies the user's saved grid edits; [`decide`] says what processing would do to an analysed
//! file; [`track`] holds the track open in the grid view and [`player`] plays it with a click;
//! [`txn`] writes a processed file through the write transaction and recovers interrupted ones.
//! Nothing here prints or knows about IPC; callers turn reports and events into text, JSON or IPC
//! messages.
#![forbid(unsafe_code)]

pub mod analyze;
pub mod batch;
pub mod cancel;
pub mod decide;
pub mod edits;
pub mod expand;
pub mod player;
pub mod session;
pub mod track;
pub mod txn;

pub use analyze::{AnalyzeReport, Analyzer, CacheStatus, Progress, REPORT_SCHEMA, Timings};
pub use batch::{
    BatchFile, BatchProgress, BatchSettings, BatchSummary, EngineEvent, default_workers, run_batch,
};
pub use cancel::CancelToken;
pub use decide::decide;
pub use edits::{EditState, apply_saved, fit_choice, save_edit};
pub use expand::{collect_audio_files, probe_all};
pub use session::{Session, run_job};
pub use track::{Track, TrackProgress};
pub use txn::{
    ApplyOptions, ApplyRequest, Place, Tag, apply_file, check_inputs, recover_at_start, undo_file,
};
