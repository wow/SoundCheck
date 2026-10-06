//! Reading and writing the parts of a FLAC file that SoundCheck rewrites, keeping every other
//! byte.
//!
//! Specifications: RFC 9639 "Free Lossless Audio Codec (FLAC)" (2024): the `fLaC` marker,
//! metadata blocks (section 8: a 4-byte header with the last-block flag, a 7-bit type and a
//! 24-bit length), STREAMINFO (8.2), PADDING (8.3), APPLICATION (8.4), SEEKTABLE (8.5),
//! `VORBIS_COMMENT` (8.6), CUESHEET (8.7), PICTURE (8.8) and frames (section 9); the Xiph
//! "Ogg Vorbis I format specification: comment field and header specification" for the
//! comment fields (a little-endian length-prefixed vendor string and `NAME=value` fields whose
//! names are ASCII 0x20..=0x7D without `=`, compared case-insensitively); the "ID3 tag version
//! 2.4.0 - Main Structure" header (tags some taggers put in front of `fLaC`), the `ID3v1` layout
//! (the last 128 bytes, starting with `TAG`) and the `APEv2` footer (`APETAGEX`) for tags that
//! follow the last frame.
//!
//! - [`read_layout`] lists the leading `ID3v2` tags, every metadata block with its type, last
//!   flag and payload range, where the frames start, and an `ID3v1`/`APEv2` tag at the end of the
//!   file. Sizes are untrusted: nothing is allocated from a declared size and nothing is read
//!   past the end of the file.
//! - [`FlacPcm`] decodes the frames to integer samples, exactly (every frame's CRC-16 checked,
//!   the decoder fed only `fLaC`, the STREAMINFO block and the frames, so unusual metadata
//!   never stops a decode), and reports where the last frame ends.
//! - [`FrameEncoder`] encodes frames of [`OUTPUT_BLOCK_FRAMES`] samples with the `flac-codec`
//!   encoder in frames-only mode and keeps what STREAMINFO and SEEKTABLE need: frame sizes and
//!   offsets, the MD5 of the samples (RFC 9639 8.2: little-endian, interleaved, whole bytes per
//!   sample) and a BLAKE3 hash of the same bytes for verification. Samples outside the output
//!   range are refused before encoding, and a frame larger than an uncompressed (verbatim)
//!   frame fails the encode.
//! - [`vorbis`], [`cuesheet`] and [`seektable`] rewrite those blocks; [`decode_frames`]
//!   decodes an output's frames again with an independent decoder (symphonia) for
//!   verification.
//!
//! [`crate::render::apply_flac`] puts these together.

pub mod cuesheet;
mod decode;
mod encode;
pub mod seektable;
mod streaminfo;
mod verify;
pub mod vorbis;
mod walk;

#[cfg(test)]
pub(crate) mod test_build;

pub use decode::{FlacPcm, frame_follows, is_frame_header};
pub use encode::{Encoded, FrameEncoder, OUTPUT_BLOCK_FRAMES, max_frame_bytes};
pub use streaminfo::{STREAMINFO_BYTES, StreamInfo};
pub use verify::{DecodedFrames, decode_frames};
pub use walk::{FlacLayout, MAX_METADATA_BLOCKS, MetadataBlock, read_layout};

/// Metadata block type numbers (RFC 9639 section 8.1, table 2).
pub mod block_type {
    /// STREAMINFO: stream parameters, always first.
    pub const STREAMINFO: u8 = 0;
    /// PADDING: zero bytes reserved for later metadata.
    pub const PADDING: u8 = 1;
    /// APPLICATION: a registered application's data.
    pub const APPLICATION: u8 = 2;
    /// SEEKTABLE: seek points.
    pub const SEEKTABLE: u8 = 3;
    /// `VORBIS_COMMENT`: the tag.
    pub const VORBIS_COMMENT: u8 = 4;
    /// CUESHEET: track and index points.
    pub const CUESHEET: u8 = 5;
    /// PICTURE: cover art and other images.
    pub const PICTURE: u8 = 6;
    /// Forbidden (it would read as a frame sync code).
    pub const FORBIDDEN: u8 = 127;
}

/// Largest metadata block payload a 24-bit length field holds, bytes.
pub const MAX_BLOCK_BYTES: u32 = (1 << 24) - 1;

/// The `fLaC` stream marker.
pub const MARKER: [u8; 4] = *b"fLaC";

/// A metadata block header: the last-block flag, the type and the 24-bit payload length.
///
/// # Panics
/// Never for lengths up to [`MAX_BLOCK_BYTES`]; callers check longer ones first.
#[must_use]
pub fn block_header(last: bool, block_type: u8, len: u32) -> [u8; 4] {
    debug_assert!(len <= MAX_BLOCK_BYTES, "metadata block lengths are checked");
    let l = len.to_be_bytes();
    [u8::from(last) << 7 | (block_type & 0x7F), l[1], l[2], l[3]]
}

/// `fLaC` and `streaminfo` as the only (last) metadata block: the start of a stream that a
/// decoder reads frames after, without the file's other blocks.
#[must_use]
pub(crate) fn stream_head(streaminfo: &[u8; STREAMINFO_BYTES]) -> Vec<u8> {
    let mut head = MARKER.to_vec();
    head.extend_from_slice(&block_header(true, block_type::STREAMINFO, 34));
    head.extend_from_slice(streaminfo);
    head
}

/// Appends `samples` as `bytes_per_sample` little-endian bytes each (the byte layout of the
/// STREAMINFO MD5, RFC 9639 section 8.2) to the cleared `out`.
pub fn le_sample_bytes(samples: &[i32], bytes_per_sample: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(samples.len() * bytes_per_sample);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes()[..bytes_per_sample]);
    }
}

#[cfg(test)]
mod tests;
