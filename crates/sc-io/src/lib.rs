//! The SoundCheck file layer.
//!
//! Decoding goes through symphonia; tags are read with lofty (read-only); analysis records are
//! cached under the user's cache directory and grid edits saved under the user's data directory. Writers are SoundCheck's own so that every tag, cover
//! and DJ-app blob survives byte for byte; they arrive with the file-layer milestone.
#![forbid(unsafe_code)]

pub mod cache;
pub mod decode;
pub mod edits;
pub mod probe;
pub mod tags;

pub use cache::Cache;
pub use decode::{Decoder, read_all};
pub use edits::{EditStore, SavedEdit};
pub use probe::probe;
