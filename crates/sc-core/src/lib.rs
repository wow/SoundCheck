//! Core types shared by every SoundCheck crate.
//!
//! Units are explicit in type names ([`Lufs`], [`Lu`], [`DbTp`], [`DbFs`], [`Bpm`],
//! [`SampleIndex`], [`Seconds`]), audio is interleaved `f32` in [`AudioBuffer`], and every
//! failure class the UI distinguishes is a variant of [`Error`]. The types in [`ipc`] cross the
//! desktop IPC boundary and export TypeScript bindings. With the `testsig` feature the crate also
//! provides deterministic synthetic signals for tests.
//!
//! This crate performs no I/O.
#![forbid(unsafe_code)]

pub mod audio;
pub mod error;
pub mod ipc;
#[cfg(feature = "testsig")]
pub mod testsig;
pub mod units;

pub use audio::{AudioBuffer, AudioSpec};
pub use error::{Error, Result};
pub use units::{Bpm, DbFs, DbTp, Lu, Lufs, SampleIndex, Seconds};

/// The SoundCheck version; identical for every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
