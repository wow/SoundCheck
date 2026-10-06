//! The FLAC walk: leading `ID3v2` tags, the `fLaC` marker, every metadata block with exact byte
//! ranges (RFC 9639 section 8), where the frames start, and a tag after the frames.
//!
//! - Leading `ID3v2` tags (some taggers put one in front of `fLaC`; libFLAC skips them) are
//!   skipped by their header size (10 bytes, a 28-bit syncsafe size, 10 more for a v2.4
//!   footer); at most [`MAX_LEADING_TAGS`]. The bytes before `fLaC` are carried as one blob.
//! - Metadata blocks are listed until the one with the last-block flag. The first must be a
//!   34-byte STREAMINFO and no other may be one; type 127 is forbidden. A block running past
//!   the end of the file is a corrupt file. At most [`MAX_METADATA_BLOCKS`] blocks.
//! - A tag after the frames: an `ID3v1` tag (the last 128 bytes start with `TAG`) and/or an
//!   `APEv2` tag before it (its 32-byte footer starts with `APETAGEX`; the size field counts the
//!   items and the footer, plus 32 header bytes when the header flag is set). This is only a
//!   hint: when STREAMINFO declares the total sample count, where the frames end is learned
//!   by decoding them, and every byte after the last frame is carried.

use std::io::{Read, Seek};
use std::ops::Range;
use std::path::Path;

use sc_core::Result;

use super::streaminfo::{STREAMINFO_BYTES, StreamInfo};
use super::{MARKER, block_type};
use crate::iff::{Source, corrupt, unsupported};

/// Most metadata blocks a stream may have.
pub const MAX_METADATA_BLOCKS: usize = 65_536;

/// Most `ID3v2` tags skipped in front of `fLaC`.
pub const MAX_LEADING_TAGS: usize = 16;

/// Length of an `ID3v1` tag, bytes.
const ID3V1_BYTES: u64 = 128;

/// Length of an `APEv2` header or footer, bytes.
const APE_FOOTER_LEN: usize = 32;

/// [`APE_FOOTER_LEN`] as a file offset.
const APE_FOOTER_BYTES: u64 = APE_FOOTER_LEN as u64;

/// One metadata block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataBlock {
    /// Block type, 0..=126.
    pub block_type: u8,
    /// Whether the header's last-block flag is set.
    pub last: bool,
    /// Byte offset of the 4-byte header.
    pub header_offset: u64,
    /// Byte range of the payload.
    pub payload: Range<u64>,
}

impl MetadataBlock {
    /// Payload length, bytes (at most 2^24 - 1).
    #[must_use]
    pub fn len(&self) -> u64 {
        self.payload.end - self.payload.start
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The block type's name (RFC 9639 table 2), `RESERVED` for the others.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self.block_type {
            block_type::STREAMINFO => "STREAMINFO",
            block_type::PADDING => "PADDING",
            block_type::APPLICATION => "APPLICATION",
            block_type::SEEKTABLE => "SEEKTABLE",
            block_type::VORBIS_COMMENT => "VORBIS_COMMENT",
            block_type::CUESHEET => "CUESHEET",
            block_type::PICTURE => "PICTURE",
            _ => "RESERVED",
        }
    }
}

/// Where everything is in a FLAC file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlacLayout {
    /// Length of the file, bytes.
    pub file_len: u64,
    /// Byte offset of `fLaC`; the bytes before it are leading tags.
    pub marker_offset: u64,
    /// Every metadata block, in file order (STREAMINFO first).
    pub blocks: Vec<MetadataBlock>,
    /// The decoded STREAMINFO.
    pub streaminfo: StreamInfo,
    /// The STREAMINFO payload as stored.
    pub streaminfo_bytes: [u8; STREAMINFO_BYTES],
    /// Byte offset of the first frame (the end of the last metadata block).
    pub frames_start: u64,
    /// Byte offset of an `ID3v1` or `APEv2` tag found at the end of the file, if any.
    pub tail_tag_start: Option<u64>,
}

impl FlacLayout {
    /// Bytes before `fLaC` (leading tags).
    #[must_use]
    pub fn leading(&self) -> Range<u64> {
        0..self.marker_offset
    }

    /// Where the frames end at the latest: the start of a tail tag, else the end of the file.
    #[must_use]
    pub fn frames_limit(&self) -> u64 {
        self.tail_tag_start.unwrap_or(self.file_len)
    }
}

/// Walks the FLAC file in `reader` (`path` names it in errors).
///
/// # Errors
/// [`sc_core::Error::UnsupportedFormat`] when there is no `fLaC` marker (after at most
/// [`MAX_LEADING_TAGS`] `ID3v2` tags) or more than [`MAX_METADATA_BLOCKS`] blocks;
/// [`sc_core::Error::Corrupt`] for a malformed leading tag, a missing or malformed
/// STREAMINFO, a second STREAMINFO, the forbidden block type 127 or a file ending inside the
/// metadata; [`sc_core::Error::Io`] when reading fails.
pub fn read_layout<R: Read + Seek>(reader: &mut R, path: &Path) -> Result<FlacLayout> {
    let mut src = Source::new(reader, path)?;
    let marker_offset = skip_leading_tags(&mut src)?;
    let mut blocks: Vec<MetadataBlock> = Vec::new();
    let mut pos = marker_offset + 4;
    loop {
        if blocks.len() >= MAX_METADATA_BLOCKS {
            return Err(unsupported(
                path,
                format!("more than {MAX_METADATA_BLOCKS} metadata blocks"),
            ));
        }
        if pos + 4 > src.len {
            return Err(corrupt(path, pos, "the file ends inside the FLAC metadata"));
        }
        let mut h = [0_u8; 4];
        src.read_at(pos, &mut h)?;
        let ty = h[0] & 0x7F;
        let len = u64::from(u32::from_be_bytes([0, h[1], h[2], h[3]]));
        let payload = pos + 4..pos + 4 + len;
        if ty == block_type::FORBIDDEN {
            return Err(corrupt(path, pos, "metadata block type 127 is forbidden"));
        }
        if payload.end > src.len {
            return Err(corrupt(
                path,
                pos,
                "a FLAC metadata block runs past the end of the file",
            ));
        }
        let first = blocks.is_empty();
        if first != (ty == block_type::STREAMINFO) {
            let why = if first {
                "the first metadata block is not STREAMINFO"
            } else {
                "a second STREAMINFO block"
            };
            return Err(corrupt(path, pos, why));
        }
        if first && len != STREAMINFO_BYTES as u64 {
            return Err(corrupt(path, pos, "STREAMINFO is not 34 bytes long"));
        }
        let last = h[0] & 0x80 != 0;
        blocks.push(MetadataBlock {
            block_type: ty,
            last,
            header_offset: pos,
            payload: payload.clone(),
        });
        pos = payload.end;
        if last {
            break;
        }
    }
    let mut streaminfo_bytes = [0_u8; STREAMINFO_BYTES];
    src.read_at(blocks[0].payload.start, &mut streaminfo_bytes)?;
    let streaminfo = StreamInfo::parse(&streaminfo_bytes);
    let tail_tag_start = tail_tag(&mut src, pos)?;
    Ok(FlacLayout {
        file_len: src.len,
        marker_offset,
        blocks,
        streaminfo,
        streaminfo_bytes,
        frames_start: pos,
        tail_tag_start,
    })
}

/// Skips `ID3v2` tags at the start; returns the offset of `fLaC`.
fn skip_leading_tags<R: Read + Seek>(src: &mut Source<'_, R>) -> Result<u64> {
    let path = src.path();
    let mut pos = 0_u64;
    for _ in 0..=MAX_LEADING_TAGS {
        if pos + 4 > src.len {
            break;
        }
        let mut magic = [0_u8; 4];
        src.read_at(pos, &mut magic)?;
        if magic == MARKER {
            return Ok(pos);
        }
        if &magic[..3] != b"ID3" || pos + 10 > src.len {
            break;
        }
        let mut h = [0_u8; 10];
        src.read_at(pos, &mut h)?;
        if h[6..10].iter().any(|b| b & 0x80 != 0) {
            return Err(corrupt(path, pos, "the ID3v2 tag size is not syncsafe"));
        }
        let size = h[6..10]
            .iter()
            .fold(0_u64, |acc, b| (acc << 7) | u64::from(*b));
        let footer = if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 };
        pos += 10 + size + footer;
    }
    Err(unsupported(
        path,
        "no FLAC stream marker (fLaC) at the start of the file or after its ID3v2 tags".into(),
    ))
}

/// The start of an `ID3v1` and/or `APEv2` tag at the end of the file, after `frames_start`.
fn tail_tag<R: Read + Seek>(src: &mut Source<'_, R>, frames_start: u64) -> Result<Option<u64>> {
    let mut end = src.len;
    let mut start = None;
    if end >= frames_start + ID3V1_BYTES {
        let mut magic = [0_u8; 3];
        src.read_at(end - ID3V1_BYTES, &mut magic)?;
        if &magic == b"TAG" {
            end -= ID3V1_BYTES;
            start = Some(end);
        }
    }
    if end >= frames_start + APE_FOOTER_BYTES {
        let mut footer = [0_u8; APE_FOOTER_LEN];
        src.read_at(end - APE_FOOTER_BYTES, &mut footer)?;
        if &footer[..8] == b"APETAGEX" {
            let le = |at: usize| {
                u64::from(u32::from_le_bytes([
                    footer[at],
                    footer[at + 1],
                    footer[at + 2],
                    footer[at + 3],
                ]))
            };
            let header = if le(20) & 0x8000_0000 != 0 {
                APE_FOOTER_BYTES
            } else {
                0
            };
            let size = le(12) + header;
            if size >= APE_FOOTER_BYTES && end - frames_start >= size {
                start = Some(end - size);
            }
        }
    }
    Ok(start)
}

#[cfg(test)]
mod tests;
