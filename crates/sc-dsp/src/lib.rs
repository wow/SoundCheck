//! Signal-processing primitives for SoundCheck.
//!
//! Everything here operates on interleaved `f32` slices with an explicit [`sc_core::AudioSpec`]
//! and is deterministic: the same input and settings give bit-identical output on every platform.
#![forbid(unsafe_code)]

pub mod biquad;
pub mod downmix;
pub mod gain;
pub mod kick_band;
pub mod onset;
pub mod resample;

pub use biquad::Biquad;
pub use downmix::to_mono;
pub use gain::{Gain, apply_gain, db_to_linear, linear_to_db};
pub use kick_band::KickBand;
pub use onset::{Onset, OnsetDetector};
pub use resample::Resampler;
