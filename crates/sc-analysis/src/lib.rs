//! Measurements over decoded audio.
//!
//! Loudness follows ITU-R BS.1770-5 and EBU R 128; beat tracking and grid fitting are documented
//! in their modules as they land. Every function is pure and deterministic.
#![forbid(unsafe_code)]

pub mod peak;

pub use peak::sample_peak;
