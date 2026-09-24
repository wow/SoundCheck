//! Signal-processing primitives for SoundCheck.
//!
//! Everything here operates on interleaved `f32` slices with an explicit [`sc_core::AudioSpec`]
//! and is deterministic: the same input and settings give bit-identical output on every platform.
#![forbid(unsafe_code)]

pub mod gain;

pub use gain::{Gain, apply_gain, db_to_linear, linear_to_db};
