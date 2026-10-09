//! The `ID3v2`.3/2.4 part of the independent reader (id3.org "ID3 tag version 2.3.0" and
//! "ID3 tag version 2.4.0 - Main Structure"): tag header, extended header, frames with their
//! flags, zero padding. Frame contents are decoded only as far as labels and text values need:
//! a v2.4 data-length indicator is skipped and v2.4 frame-level unsynchronisation is undone
//! before reading a description or value; the listed bytes are always the stored bytes. An
//! `ID3v2`.2 tag (three-character frame ids, id3.org "ID3 tag version 2") is only checked for its
//! header and size and lists no items: it is never edited, so its chunk is compared whole.

use super::parse::{Block, Kind, slice, to_usize};

/// The header of an `ID3v2` tag found in a chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id3Tag {
    /// Index in `Parsed::blocks` of the chunk holding the tag.
    pub chunk: usize,
    /// Major version: 2, 3 or 4.
    pub version: u8,
    /// Revision byte.
    pub revision: u8,
    /// Header flags byte (0x80 unsynchronisation, 0x40 extended header, 0x10 footer).
    pub flags: u8,
    /// Tag size from the header (excludes the 10-byte header), bytes.
    pub size: usize,
    /// Zero bytes after the last frame, bytes.
    pub padding: usize,
}

/// Decodes a 28-bit syncsafe integer.
///
/// # Errors
/// When a byte has its high bit set.
pub fn syncsafe(b: &[u8]) -> Result<usize, String> {
    if b.iter().any(|x| x & 0x80 != 0) {
        return Err("syncsafe integer with the high bit set".into());
    }
    Ok(b.iter().fold(0, |acc, x| (acc << 7) | usize::from(*x)))
}

fn be32(b: &[u8], at: usize) -> Result<usize, String> {
    let s = slice(b, at, 4)?;
    to_usize(u64::from(u32::from_be_bytes([s[0], s[1], s[2], s[3]])))
}

/// Parses the `ID3v2` tag at the start of `p` (`base` is its file offset; `chunk` the index of
/// the chunk block); returns the header and the extended header and frame blocks.
///
/// # Errors
/// Structural problems, a v2.3 tag with tag-level unsynchronisation (its frames cannot be
/// walked without undoing it), or non-zero padding.
pub fn parse_id3(p: &[u8], base: usize, chunk: usize) -> Result<(Id3Tag, Vec<Block>), String> {
    if p.get(..3) != Some(b"ID3") {
        return Err("ID3 chunk does not start with an ID3v2 header".into());
    }
    let head = slice(p, 3, 3)?;
    let (version, revision, flags) = (head[0], head[1], head[2]);
    let size = syncsafe(slice(p, 6, 4)?)?;
    if !(2..=4).contains(&version) {
        return Err(format!("ID3v2.{version} is not handled"));
    }
    if version == 3 && flags & 0x80 != 0 {
        return Err("unsynchronised v2.3 tags are not walked frame by frame".into());
    }
    let end = 10 + size;
    if end > p.len() {
        return Err("ID3 tag larger than its chunk".into());
    }
    if version == 2 {
        let tag = Id3Tag {
            chunk,
            version,
            revision,
            flags,
            size,
            padding: 0,
        };
        return Ok((tag, Vec::new()));
    }
    let mut items = Vec::new();
    let mut pos = 10;
    if flags & 0x40 != 0 {
        let len = if version == 3 {
            4 + be32(p, 10)?
        } else {
            syncsafe(slice(p, 10, 4)?)?
        };
        if pos + len > end {
            return Err("ID3 extended header larger than the tag".into());
        }
        items.push(Block {
            kind: Kind::Id3ExtHeader,
            id: "ext-header".into(),
            offset: base + pos,
            bytes: p[pos..pos + len].to_vec(),
            pad: None,
        });
        pos += len;
    }
    while pos + 10 <= end && p[pos] != 0 {
        let id = slice(p, pos, 4)?;
        if !id
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(format!("bad frame id at {pos}"));
        }
        let body_len = if version == 3 {
            be32(p, pos + 4)?
        } else {
            syncsafe(slice(p, pos + 4, 4)?)?
        };
        let frame = slice(p, pos, 10 + body_len)?;
        if pos + frame.len() > end {
            return Err(format!("frame at {pos} runs past the tag"));
        }
        let id = String::from_utf8_lossy(id).into_owned();
        let content = content(version, frame[9], &frame[10..]);
        items.push(Block {
            id: frame_label(&id, &content),
            kind: Kind::Id3Frame,
            offset: base + pos,
            bytes: frame.to_vec(),
            pad: None,
        });
        pos += frame.len();
    }
    if p[pos..end].iter().any(|x| *x != 0) {
        return Err("non-zero bytes in the ID3 padding".into());
    }
    let tag = Id3Tag {
        chunk,
        version,
        revision,
        flags,
        size,
        padding: end - pos,
    };
    Ok((tag, items))
}

/// A frame body with a v2.4 data-length indicator skipped and unsynchronisation undone.
fn content(version: u8, format_flags: u8, body: &[u8]) -> Vec<u8> {
    if version != 4 {
        return body.to_vec();
    }
    let body = if format_flags & 0x01 != 0 {
        body.get(4..).unwrap_or_default()
    } else {
        body
    };
    if format_flags & 0x02 == 0 {
        return body.to_vec();
    }
    let mut out = Vec::with_capacity(body.len());
    let mut previous_ff = false;
    for &b in body {
        if !(previous_ff && b == 0) {
            out.push(b);
        }
        previous_ff = b == 0xFF;
    }
    out
}

/// Decodes ID3 text in `encoding` up to its terminator; returns the text and the bytes used
/// (terminator included).
#[must_use]
pub fn id3_string(encoding: u8, b: &[u8]) -> (String, usize) {
    if encoding == 1 || encoding == 2 {
        let mut units = Vec::new();
        let mut i = 0;
        while i + 1 < b.len() {
            let u = if encoding == 2 {
                u16::from_be_bytes([b[i], b[i + 1]])
            } else {
                u16::from_le_bytes([b[i], b[i + 1]])
            };
            i += 2;
            if u == 0 {
                break;
            }
            units.push(u);
        }
        // A byte-order mark selects the order for encoding 1.
        let units: Vec<u16> = match units.first() {
            Some(0xFEFF) => units[1..].to_vec(),
            Some(0xFFFE) => units[1..].iter().map(|u| u.swap_bytes()).collect(),
            _ => units,
        };
        (String::from_utf16_lossy(&units), i)
    } else {
        let n = b.iter().position(|x| *x == 0).unwrap_or(b.len());
        let text = if encoding == 3 {
            String::from_utf8_lossy(&b[..n]).into_owned()
        } else {
            b[..n].iter().map(|c| char::from(*c)).collect()
        };
        (text, (n + 1).min(b.len()))
    }
}

/// Frame label: the id, plus `:description` for TXXX/WXXX/GEOB/PRIV and non-empty COMM.
fn frame_label(id: &str, body: &[u8]) -> String {
    let Some((&enc, rest)) = body.split_first() else {
        return id.into();
    };
    let desc = match id {
        "TXXX" | "WXXX" => id3_string(enc, rest).0,
        "PRIV" => id3_string(0, body).0,
        "COMM" if rest.len() >= 3 => id3_string(enc, &rest[3..]).0,
        "GEOB" => {
            let (_, mime) = id3_string(0, rest);
            let (_, file) = id3_string(enc, &rest[mime..]);
            id3_string(enc, &rest[mime + file..]).0
        }
        _ => String::new(),
    };
    if desc.is_empty() {
        id.into()
    } else {
        format!("{id}:{desc}")
    }
}

/// The text encoding byte and value of a text frame (`T***`) or a `TXXX` frame of a tag of
/// `version`, without terminator.
#[must_use]
pub fn frame_text(version: u8, frame: &Block) -> Option<(u8, String)> {
    let id = frame.bytes.get(..4)?;
    let body = content(version, *frame.bytes.get(9)?, frame.bytes.get(10..)?);
    let (&enc, rest) = body.split_first()?;
    if id == b"TXXX" {
        let (_, used) = id3_string(enc, rest);
        return Some((enc, id3_string(enc, &rest[used..]).0));
    }
    (id[0] == b'T').then(|| (enc, id3_string(enc, rest).0))
}
