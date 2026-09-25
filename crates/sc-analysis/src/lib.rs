//! Measurements over decoded audio.
//!
//! Loudness follows ITU-R BS.1770-5 and EBU R 128 ([`loudness`]); beat tracking and grid fitting
//! are documented in their modules as they land. Every function is pure and deterministic.
#![forbid(unsafe_code)]

pub mod beats;
pub mod grid;
pub mod loudness;
pub mod meter;
pub mod peak;

pub use beats::{BeatTracker, RawBeats};
pub use loudness::{LoudnessMeter, measure};
pub use peak::sample_peak;
