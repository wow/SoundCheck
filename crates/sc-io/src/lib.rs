//! The SoundCheck file layer.
//!
//! Decoding goes through symphonia; tags are read with lofty (read-only); analysis records are
//! cached under the user's cache directory and grid edits saved under the user's data directory.
//! [`iff`] reads WAV, RF64, AIFF and AIFF-C files exactly (chunk table with pad bytes and
//! trailing bytes, audio format, streaming PCM), including RF64, which symphonia does not read.
//! Writers are SoundCheck's own so that every tag, cover and DJ-app blob survives byte for byte:
//! [`render::apply_iff`] renders a WAV/AIFF file with a new level and an optional head trim into
//! a DJ-safe file that carries every other chunk, and [`id3`] edits SoundCheck's frames into an
//! `ID3v2` tag while keeping every other frame's bytes.
#![forbid(unsafe_code)]

pub mod cache;
pub mod decode;
pub mod edits;
pub mod flac;
pub mod id3;
pub mod iff;
pub mod probe;
pub mod render;
pub mod tags;

pub use cache::Cache;
pub use decode::{Decoder, read_all};
pub use edits::{EditStore, SavedEdit};
pub use probe::probe;
pub use render::{RenderReport, apply_flac, apply_iff};
