//! The SoundCheck engine: the one pipeline behind both the desktop app and `sc-cli`.
//!
//! [`analyze`] runs one file end to end (decode, loudness, beats, onsets, meter, grid, tags,
//! cache); [`batch`] runs many on worker threads with progress and cancellation, analysing or
//! exporting them; [`edits`] applies the user's saved grid edits and carries them over to
//! exported files; [`decide`] says what processing would do to an analysed file and [`export`]
//! what exporting does to it; [`track`] holds the track open in the grid view and [`player`]
//! plays it with a click; [`txn`] writes a processed file through the write transaction and
//! recovers interrupted ones, gating exports until that recovery has run. Nothing here prints or knows about IPC; callers
//! turn reports and events into text, JSON or IPC messages.
#![forbid(unsafe_code)]

pub mod analyze;
pub mod batch;
pub mod cancel;
pub mod decide;
pub mod edits;
pub mod expand;
pub mod export;
pub mod player;
pub mod session;
pub mod track;
pub mod txn;

pub use analyze::{AnalyzeReport, Analyzer, CacheStatus, Progress, REPORT_SCHEMA, Timings};
pub use batch::{
    BatchFile, BatchProgress, BatchSettings, BatchSummary, EngineEvent, Task, default_workers,
    run_batch,
};
pub use cancel::CancelToken;
pub use decide::decide;
pub use edits::{EditState, apply_saved, carry_edit, fit_choice, save_edit};
pub use expand::{collect_audio_files, probe_all};
pub use export::{
    Artefacts, BatchRow, ExportInput, ExportSource, ProcessDone, ProcessSettings, RowAction,
    RowTrack, SeratoPresence, XmlSelect, plan_export, plan_export_snapped, plan_snapped_cut,
    write_artefacts,
};
pub use session::{Session, run_job};
pub use track::{Track, TrackProgress};
pub use txn::{
    ApplyOptions, ApplyRequest, Place, RecoveryGate, RecoveryGuard, Tag, apply_file, check_inputs,
    forget_cached, forget_recovered, recover_at_start, undo_file,
};
