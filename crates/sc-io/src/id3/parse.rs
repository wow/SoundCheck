//! Indexing an `ID3v2`.3/2.4 tag: header, extended header and frames by byte range, checked
//! against the bytes at hand (ID3v2.3.0 sections 3.1-3.3, ID3v2.4.0 Main Structure sections
//! 3.1-3.4 and 4).

use std::ops::Range;

use super::text::{FrameHead, frame_head, txxx_value};
use super::{MAX_FRAMES, MAX_TAG_BYTES, NotEditable};

/// Bytes of the tag header, of a frame header and of a v2.4 footer.
pub(crate) const HEADER_BYTES: usize = 10;

/// Header flag: tag-level unsynchronisation.
pub(crate) const FLAG_UNSYNC: u8 = 0x80;
/// Header flag: an extended header follows the header.
pub(crate) const FLAG_EXTENDED: u8 = 0x40;
/// Header flag (v2.4): a footer follows the padding.
pub(crate) const FLAG_FOOTER: u8 = 0x10;

/// Header flags each version defines: v2.3 unsynchronisation, extended header, experimental;
/// v2.4 adds the footer.
fn defined_flags(major: u8) -> u8 {
    if major == 3 { 0xE0 } else { 0xF0 }
}

/// Decodes a 28-bit syncsafe integer (four bytes of 7 bits, most significant first); `None`
/// when a byte has its high bit set.
#[must_use]
pub fn decode_syncsafe(b: [u8; 4]) -> Option<u32> {
    if b.iter().any(|x| x & 0x80 != 0) {
        return None;
    }
    Some(b.iter().fold(0, |acc, x| (acc << 7) | u32::from(*x)))
}

/// Encodes `v` as a 28-bit syncsafe integer; `None` when `v` needs more than 28 bits.
#[must_use]
pub fn encode_syncsafe(v: u32) -> Option<[u8; 4]> {
    if v >= 1 << 28 {
        return None;
    }
    // Each byte is masked to 7 bits, so the narrowing is exact.
    #[allow(clippy::cast_possible_truncation)]
    let b = |shift: u32| ((v >> shift) & 0x7F) as u8;
    Some([b(21), b(14), b(7), b(0)])
}

/// One frame of a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRef {
    /// Four-character frame id (`A-Z`, `0-9`).
    pub id: [u8; 4],
    /// The two flag bytes (status, format).
    pub flags: [u8; 2],
    /// Header and body, byte offsets into the tag.
    pub range: Range<usize>,
    /// The description of a `TXXX` frame, the short content description of a `COMM` frame or
    /// the content description of a `GEOB` frame, when it can be read (not compressed or
    /// encrypted, a known text encoding; UTF-16 without a byte-order mark read little-endian).
    /// `None` for every other frame.
    pub description: Option<String>,
    /// The same description read big-endian, for UTF-16 without a byte-order mark (the
    /// writer's byte order is unknown, so both readings count).
    pub description_be: Option<String>,
    /// The text encoding byte (0-3) of a `TXXX`, `COMM` or `GEOB` frame whose description is
    /// read.
    pub encoding: Option<u8>,
    /// The language of a `COMM` frame (ISO 639-2, three bytes as stored, e.g. `eng`).
    pub language: Option<[u8; 3]>,
    /// The MIME type of a `GEOB` frame.
    pub mime: Option<String>,
    /// The file name of a `GEOB` frame.
    pub file_name: Option<String>,
}

impl FrameRef {
    fn new(id: [u8; 4], flags: [u8; 2], range: Range<usize>, head: Option<FrameHead>) -> Self {
        let mut frame = Self {
            id,
            flags,
            range,
            description: None,
            description_be: None,
            encoding: None,
            language: None,
            mime: None,
            file_name: None,
        };
        if let Some(h) = head {
            frame.description = Some(h.description);
            frame.description_be = h.description_be;
            frame.encoding = Some(h.encoding);
            frame.language = h.language;
            frame.mime = h.mime;
            frame.file_name = h.file_name;
        }
        frame
    }

    /// Whether this is a frame `id` whose description (either reading) equals `description`,
    /// compared ASCII case-insensitively.
    #[must_use]
    pub fn is(&self, id: &[u8; 4], description: &str) -> bool {
        let same = |d: &Option<String>| {
            d.as_ref()
                .is_some_and(|d| d.eq_ignore_ascii_case(description))
        };
        &self.id == id && (same(&self.description) || same(&self.description_be))
    }
}

/// The layout of a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagIndex {
    /// Major version: 3 or 4.
    pub major: u8,
    /// Revision byte.
    pub revision: u8,
    /// Header flags byte.
    pub flags: u8,
    /// The extended header, byte offsets into the tag.
    pub extended: Option<Range<usize>>,
    /// Every frame in tag order.
    pub frames: Vec<FrameRef>,
    /// The zero padding after the last frame, byte offsets into the tag.
    pub padding: Range<usize>,
    /// Whether a 10-byte footer follows the padding (v2.4).
    pub footer: bool,
    /// End of the tag: header, size field and footer, bytes.
    pub end: usize,
}

impl TagIndex {
    /// The value of the `TXXX` frame `frame` of `tag` (the bytes this index was parsed from):
    /// the text after its description. `None` for another frame or one that cannot be read
    /// (compressed, encrypted, an unknown text encoding).
    #[must_use]
    pub fn txxx_value(&self, tag: &[u8], frame: &FrameRef) -> Option<String> {
        if &frame.id != b"TXXX" {
            return None;
        }
        let body = tag.get(frame.range.start + HEADER_BYTES..frame.range.end)?;
        txxx_value(self.major, frame.flags, body)
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn syncsafe_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    decode_syncsafe([s[0], s[1], s[2], s[3]])
}

fn to_usize(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Indexes the `ID3v2` tag at the start of `bytes` (which may continue past the tag's end).
///
/// # Errors
/// [`NotEditable`] when the tag is not version 2.3 or 2.4, is unsynchronised, has undefined
/// header flags, a CRC or restrictions in its extended header, is larger than
/// [`MAX_TAG_BYTES`], or does not parse.
pub fn parse_tag(bytes: &[u8]) -> Result<TagIndex, NotEditable> {
    if bytes.len() < HEADER_BYTES || &bytes[..3] != b"ID3" {
        return Err(NotEditable::malformed("no ID3v2 header"));
    }
    let (major, revision, flags) = (bytes[3], bytes[4], bytes[5]);
    if major == 0xFF || revision == 0xFF {
        return Err(NotEditable::malformed("version byte 0xFF"));
    }
    if major != 3 && major != 4 {
        return Err(NotEditable::UnsupportedVersion { major });
    }
    let size =
        syncsafe_at(bytes, 6).ok_or_else(|| NotEditable::malformed("tag size not syncsafe"))?;
    let footer = major == 4 && flags & FLAG_FOOTER != 0;
    let area_end = HEADER_BYTES + to_usize(size);
    let end = area_end + if footer { HEADER_BYTES } else { 0 };
    if end as u64 > MAX_TAG_BYTES {
        return Err(NotEditable::TooLarge { bytes: end as u64 });
    }
    if flags & FLAG_UNSYNC != 0 {
        return Err(NotEditable::Unsynchronised);
    }
    if flags & !defined_flags(major) != 0 {
        return Err(NotEditable::UnknownFlags { flags });
    }
    if end > bytes.len() {
        return Err(NotEditable::malformed(format!(
            "the tag declares {end} bytes, its chunk holds {}",
            bytes.len()
        )));
    }
    let extended = if flags & FLAG_EXTENDED != 0 {
        Some(extended_header(bytes, major, area_end)?)
    } else {
        None
    };
    let first = extended.as_ref().map_or(HEADER_BYTES, |r| r.end);
    let (frames, last) = walk_frames(bytes, major, first, area_end)?;
    if bytes[last..area_end].iter().any(|b| *b != 0) {
        return Err(NotEditable::malformed(format!(
            "non-zero bytes in the padding at byte {last}"
        )));
    }
    if major == 4 {
        check_plain_sizes(bytes, &frames, last, area_end)?;
    }
    if footer {
        if &bytes[area_end..area_end + 3] != b"3DI" {
            return Err(NotEditable::malformed("the footer does not start with 3DI"));
        }
        if bytes[area_end + 3..area_end + HEADER_BYTES] != bytes[3..HEADER_BYTES] {
            return Err(NotEditable::malformed(
                "the footer does not repeat the header",
            ));
        }
    }
    Ok(TagIndex {
        major,
        revision,
        flags,
        extended,
        frames,
        padding: last..area_end,
        footer,
        end,
    })
}

/// The extended header's byte range: v2.3 a plain size excluding itself (6 or 10), flags with
/// 0x8000 = CRC, the padding size; v2.4 a syncsafe size including itself, one flag byte count
/// (1), flags with 0x20 = CRC and 0x10 = restrictions, then the flags' data.
fn extended_header(bytes: &[u8], major: u8, area_end: usize) -> Result<Range<usize>, NotEditable> {
    let short = || NotEditable::malformed("extended header past the end of the tag");
    let start = HEADER_BYTES;
    let len = if major == 3 {
        let n = be32(bytes, start).ok_or_else(short)?;
        if n < 6 {
            return Err(NotEditable::malformed(format!(
                "v2.3 extended header size {n}"
            )));
        }
        to_usize(n).saturating_add(4)
    } else {
        let n = syncsafe_at(bytes, start)
            .ok_or_else(|| NotEditable::malformed("extended header size"))?;
        if n < 6 {
            return Err(NotEditable::malformed(format!(
                "v2.4 extended header size {n}"
            )));
        }
        to_usize(n)
    };
    let end = start
        .checked_add(len)
        .filter(|e| *e <= area_end)
        .ok_or_else(short)?;
    let ext = &bytes[start..end];
    if major == 3 {
        if ext[4] & 0x80 != 0 {
            return Err(NotEditable::ExtendedHeaderCrc);
        }
    } else {
        if ext[4] != 1 {
            return Err(NotEditable::malformed(format!(
                "{} extended flag bytes",
                ext[4]
            )));
        }
        if ext[5] & 0x20 != 0 {
            return Err(NotEditable::ExtendedHeaderCrc);
        }
        if ext[5] & 0x10 != 0 {
            return Err(NotEditable::Restricted);
        }
    }
    Ok(start..end)
}

/// Whether a frame holds text (`T***`, `W***`, `COMM`, `USLT`, `USER`), which no writer ends
/// with a run of 128 or more zero bytes.
fn is_text_frame(id: [u8; 4]) -> bool {
    matches!(id[0], b'T' | b'W') || matches!(&id, b"COMM" | b"USLT" | b"USER")
}

/// Refuses a v2.4 tag whose frame sizes read two ways. Some writers (old iTunes versions)
/// stored v2.4 frame sizes as plain 32-bit integers. A size of 128 bytes or more then reads
/// smaller as syncsafe, and when the rest of the frame is zeros the syncsafe walk takes it for
/// padding, where an edit would write. Like the `TagLib` library, the syncsafe reading is trusted when a
/// frame header follows the frame (only the last frame can end on padding). For the last frame,
/// when the plain reading also stays inside the tag (its tail is then all zeros), both
/// readings fit; that is accepted for a text frame (no writer ends text with 128 zero bytes)
/// and refused for any other frame.
fn check_plain_sizes(
    bytes: &[u8],
    frames: &[FrameRef],
    padding_start: usize,
    area_end: usize,
) -> Result<(), NotEditable> {
    let Some(f) = frames.last() else {
        return Ok(());
    };
    let syncsafe = f.range.len() - HEADER_BYTES;
    let plain = be32(bytes, f.range.start + 4).map_or(usize::MAX, to_usize);
    if padding_start == area_end || plain == syncsafe || is_text_frame(f.id) {
        return Ok(());
    }
    let plain_end = (f.range.start + HEADER_BYTES).checked_add(plain);
    if plain_end.is_some_and(|e| e <= area_end) {
        return Err(NotEditable::malformed(format!(
            "ambiguous frame sizes: the frame at byte {} holds {syncsafe} bytes as syncsafe and \
             {plain} as a plain integer",
            f.range.start
        )));
    }
    Ok(())
}

/// Walks the frames from `pos` up to the first zero byte (padding) or the end of the frame
/// area; returns them and where they end.
fn walk_frames(
    bytes: &[u8],
    major: u8,
    mut pos: usize,
    area_end: usize,
) -> Result<(Vec<FrameRef>, usize), NotEditable> {
    let mut frames = Vec::new();
    while pos + HEADER_BYTES <= area_end && bytes[pos] != 0 {
        if frames.len() == MAX_FRAMES {
            return Err(NotEditable::malformed(format!(
                "more than {MAX_FRAMES} frames"
            )));
        }
        let id = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        if !id
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(NotEditable::malformed(format!(
                "invalid frame id at byte {pos}"
            )));
        }
        let body_len = if major == 3 {
            be32(bytes, pos + 4)
        } else {
            syncsafe_at(bytes, pos + 4)
        }
        .ok_or_else(|| NotEditable::malformed(format!("frame size not syncsafe at byte {pos}")))?;
        let end = (pos + HEADER_BYTES)
            .checked_add(to_usize(body_len))
            .filter(|e| *e <= area_end)
            .ok_or_else(|| {
                NotEditable::malformed(format!("the frame at byte {pos} runs past the tag"))
            })?;
        let flags = [bytes[pos + 8], bytes[pos + 9]];
        let head = frame_head(major, id, flags, &bytes[pos + HEADER_BYTES..end]);
        frames.push(FrameRef::new(id, flags, pos..end, head));
        pos = end;
    }
    Ok((frames, pos))
}

#[cfg(test)]
mod tests;
