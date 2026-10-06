//! Rebuilding a tag with SoundCheck's frames replaced or appended and every other byte kept.

use super::parse::{HEADER_BYTES, encode_syncsafe, parse_tag};
use super::text::{Edit, Label};
use super::{GROWTH_PADDING_BYTES, MAX_TAG_BYTES, NotEditable};

/// What [`edit_tag`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditSummary {
    /// Tag major version (3 or 4), unchanged.
    pub major: u8,
    /// Existing frames replaced in place by one of ours.
    pub replaced: u32,
    /// Frames appended after the last existing frame.
    pub appended: u32,
    /// Whether the tag grew (its padding could not hold the new frames).
    pub grew: bool,
    /// Zero padding in the new tag, bytes.
    pub padding_bytes: u32,
    /// The new tag's length (header, extended header, frames, padding, footer), bytes.
    pub tag_bytes: u32,
}

/// The rebuilt chunk payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    /// The new tag followed by the bytes that followed the old tag in its chunk.
    pub bytes: Vec<u8>,
    /// What changed.
    pub summary: EditSummary,
}

/// Applies `edits` to the tag at the start of `chunk` (an `id3 `/`ID3 ` chunk payload).
///
/// # Errors
/// [`NotEditable`] when the tag cannot be edited safely (see [`super::parse_tag`]), would grow
/// past [`MAX_TAG_BYTES`], or holds a `TXXX` frame whose description cannot be read while a
/// `TXXX` edit is requested (it might be one of ours, and appending would duplicate it).
pub fn edit_tag(chunk: &[u8], edits: &[Edit]) -> Result<Edited, NotEditable> {
    let index = parse_tag(chunk)?;
    let txxx_edit = edits.iter().any(|e| matches!(e.label(), Label::Txxx(_)));
    let unreadable = index
        .frames
        .iter()
        .any(|f| &f.id == b"TXXX" && f.description.is_none());
    if txxx_edit && unreadable {
        return Err(NotEditable::UnreadableTxxx);
    }
    let major = index.major;
    let mut frames = Vec::with_capacity(index.padding.end - HEADER_BYTES);
    let mut replaced = 0_u32;
    for f in &index.frames {
        match edits.iter().find(|e| e.matches(f)) {
            Some(edit) => {
                frames.extend(edit.frame(major));
                replaced += 1;
            }
            None => frames.extend_from_slice(&chunk[f.range.clone()]),
        }
    }
    let mut appended = 0_u32;
    for edit in edits {
        if !index.frames.iter().any(|f| edit.matches(f)) {
            frames.extend(edit.frame(major));
            appended += 1;
        }
    }
    let first = index.extended.as_ref().map_or(HEADER_BYTES, |r| r.end);
    let room = index.padding.end - first;
    let (padding, grew) = match (index.footer, room.checked_sub(frames.len())) {
        (true, left) => (0, left.is_none()),
        (false, Some(left)) => (left, false),
        (false, None) => (GROWTH_PADDING_BYTES, true),
    };
    let ext = index
        .extended
        .as_ref()
        .map_or(&[][..], |r| &chunk[r.clone()]);
    let size = ext.len() + frames.len() + padding;
    let footer_bytes = if index.footer { HEADER_BYTES } else { 0 };
    let tag_bytes = HEADER_BYTES + size + footer_bytes;
    let too_large = || NotEditable::TooLarge {
        bytes: tag_bytes as u64,
    };
    if tag_bytes as u64 > MAX_TAG_BYTES {
        return Err(too_large());
    }
    let size_field = u32::try_from(size)
        .ok()
        .and_then(encode_syncsafe)
        .ok_or_else(too_large)?;
    let padding_field = u32::try_from(padding).map_err(|_| too_large())?;
    let rest = &chunk[index.end..];
    let mut out = Vec::with_capacity(tag_bytes + rest.len());
    out.extend_from_slice(&chunk[..6]);
    out.extend_from_slice(&size_field);
    out.extend_from_slice(ext);
    if major == 3 && !ext.is_empty() {
        // v2.3 extended header: bytes 6..10 hold the padding size (checked to be present: a
        // v2.3 extended header has at least 10 bytes).
        let at = HEADER_BYTES + 6;
        out[at..at + 4].copy_from_slice(&padding_field.to_be_bytes());
    }
    out.extend_from_slice(&frames);
    out.resize(out.len() + padding, 0);
    if index.footer {
        out.extend_from_slice(b"3DI");
        out.extend_from_slice(&[major, index.revision, index.flags]);
        out.extend_from_slice(&size_field);
    }
    out.extend_from_slice(rest);
    Ok(Edited {
        bytes: out,
        summary: EditSummary {
            major,
            replaced,
            appended,
            grew,
            padding_bytes: padding_field,
            tag_bytes: u32::try_from(tag_bytes).map_err(|_| too_large())?,
        },
    })
}

#[cfg(test)]
mod tests;
