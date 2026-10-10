//! Core types shared by every SoundCheck crate.
//!
//! Units are explicit in type names ([`Lufs`], [`Lu`], [`DbTp`], [`DbFs`], [`Bpm`],
//! [`SampleIndex`], [`Seconds`]), audio is interleaved `f32` in [`AudioBuffer`], and every
//! failure class the UI distinguishes is a variant of [`Error`]. The types in [`ipc`] cross the
//! desktop IPC boundary and export TypeScript bindings, as do the [`analysis`] record types and
//! the [`plan`] types (what processing would do) and the [`export`] types (what exporting a batch
//! does to each file); [`render`] holds what a lossless render is asked to do. With the `testsig`
//! feature the crate also provides deterministic synthetic signals for tests.
//!
//! This crate performs no I/O.
#![forbid(unsafe_code)]

pub mod analysis;
pub mod audio;
pub mod error;
pub mod export;
pub mod ipc;
pub mod plan;
pub mod render;
#[cfg(feature = "testsig")]
pub mod testsig;
pub mod units;

pub use analysis::{
    AnalysisRecord, AnalysisSettings, Confidence, Grid, GridEdit, GridEvidence, LoudnessReport,
    Meter, OnsetList, Reason, TagHints, Verdict,
};
pub use audio::{AudioBuffer, AudioSpec};
pub use error::{ChangeCause, Error, InPlaceRefusal, Result};
pub use render::{BextLoudness, RenderRequest, Tag, TagEdit};
pub use units::{Bpm, DbFs, DbTp, Lu, Lufs, SampleIndex, Seconds};

/// The SoundCheck version; identical for every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
