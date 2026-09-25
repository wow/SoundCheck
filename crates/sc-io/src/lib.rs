//! The SoundCheck file layer.
//!
//! Decoding goes through symphonia. Writers are SoundCheck's own so that every tag, cover and
//! DJ-app blob survives byte for byte; they arrive with the file-layer milestone.
#![forbid(unsafe_code)]

pub mod decode;

pub use decode::{Decoder, read_all};
