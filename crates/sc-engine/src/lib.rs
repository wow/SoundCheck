//! The SoundCheck engine: the one pipeline behind both the desktop app and `sc-cli`.
//!
//! [`analyze`] runs one file end to end (decode, loudness, beats, onsets, meter, grid, tags,
//! cache). Nothing here prints or knows about IPC; callers turn reports into text, JSON or events.

pub mod analyze;

pub use analyze::{AnalyzeReport, Analyzer, CacheStatus, REPORT_SCHEMA, Timings};
