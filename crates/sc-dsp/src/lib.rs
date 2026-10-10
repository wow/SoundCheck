//! Signal-processing primitives for SoundCheck.
//!
//! Everything here operates on interleaved `f32` slices with an explicit [`sc_core::AudioSpec`]
//! and is deterministic: the same input and settings give bit-identical output on every platform.
#![forbid(unsafe_code)]

pub mod biquad;
pub mod dither;
pub mod downmix;
pub mod fade;
pub mod gain;
pub mod kick_band;
pub mod onset;
pub mod requantise;
pub mod resample;
pub mod stream_resample;

pub use biquad::Biquad;
pub use dither::Tpdf;
pub use downmix::to_mono;
pub use fade::{FadeIn, raised_cosine_in};
pub use gain::{Gain, apply_gain, db_to_linear, linear_to_db};
pub use kick_band::KickBand;
pub use onset::{Onset, OnsetDetector};
pub use requantise::{Requantiser, SourceDepth};
pub use resample::Resampler;
pub use stream_resample::StreamResampler;
