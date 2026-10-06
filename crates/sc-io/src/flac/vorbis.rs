//! Re-emitting a FLAC `VORBIS_COMMENT` block (RFC 9639 section 8.6) with SoundCheck's fields,
//! keeping every other byte.
//!
//! The payload is the Vorbis comment header without the framing bit (Xiph "Ogg Vorbis I format
//! specification", section 5): a 32-bit little-endian vendor length and the vendor string, a
//! 32-bit little-endian field count, and per field a 32-bit little-endian length and the
//! field, `NAME=value` in UTF-8. Field names are ASCII 0x20..=0x7D except `=` and compare
//! case-insensitively.
//!
//! [`edit_comment`] keeps the vendor string byte for byte and every field byte for byte and in
//! order, except that a field whose name matches an edit (ASCII case-insensitive) gets the
//! edit's value in place, under its own name as written (every such field if there are
//! several); the other edits are appended once after the last field, in request order. A
//! payload that does not parse exactly (a length running past the block, bytes after the last
//! field) or a result too large for a metadata block is not edited ([`NotEditable`]); the
//! block is then carried unchanged.

use std::ops::Range;

use sc_core::{Error, Result, TagEdit};

use super::MAX_BLOCK_BYTES;
use crate::id3::{MAX_VALUE_BYTES, NotEditable};

/// One field SoundCheck writes: a name and its value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VorbisEdit {
    name: String,
    value: String,
}

fn invalid(what: &str) -> Error {
    Error::InvalidArgument(format!("Vorbis comment edit: {what}"))
}

impl VorbisEdit {
    /// An edit of the field `name` (ASCII 0x20..=0x7D except `=`, not empty) to `value`.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for an invalid name, a NUL character in the value, or a
    /// value over [`MAX_VALUE_BYTES`].
    pub fn new(name: &str, value: &str) -> Result<Self> {
        let valid = !name.is_empty() && name.bytes().all(|b| (0x20..=0x7D).contains(&b) && b != b'=');
        if !valid {
            return Err(invalid(&format!(
                "{name:?} is not a field name (ASCII 0x20..=0x7D without '=')"
            )));
        }
        if value.contains('\0') {
            return Err(invalid(&format!("the value of {name:?} holds a NUL character")));
        }
        if value.len() > MAX_VALUE_BYTES {
            return Err(invalid(&format!(
                "the value of {name:?} has {} bytes, at most {MAX_VALUE_BYTES}",
                value.len()
            )));
        }
        Ok(Self {
            name: name.to_string(),
            value: value.to_string(),
        })
    }

    /// The field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether this edit replaces a field called `name` (ASCII case-insensitive).
    #[must_use]
    pub fn matches(&self, name: &[u8]) -> bool {
        name.eq_ignore_ascii_case(self.name.as_bytes())
    }
}

/// Validated edits from a render request.
///
/// # Errors
/// [`Error::InvalidArgument`] for an invalid edit (see [`VorbisEdit::new`]) or two edits of
/// the same field.
pub fn edits_from(edits: &[TagEdit]) -> Result<Vec<VorbisEdit>> {
    let mut out: Vec<VorbisEdit> = Vec::with_capacity(edits.len());
    for e in edits {
        let edit = VorbisEdit::new(&e.label, &e.value)?;
        if out.iter().any(|o| o.matches(edit.name.as_bytes())) {
            return Err(invalid(&format!("{:?} is edited twice", e.label)));
        }
        out.push(edit);
    }
    Ok(out)
}

/// Where the parts of a comment payload are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentIndex {
    /// The vendor length field and string.
    pub vendor: Range<usize>,
    /// Each field without its length prefix, in order.
    pub fields: Vec<Range<usize>>,
}

impl CommentIndex {
    /// The name of the field at `range` of `payload` (the bytes before the first `=`), or
    /// `None` for a field without `=`.
    #[must_use]
    pub fn name<'p>(payload: &'p [u8], range: &Range<usize>) -> Option<&'p [u8]> {
        let field = &payload[range.clone()];
        field.iter().position(|b| *b == b'=').map(|eq| &field[..eq])
    }
}

fn le32(p: &[u8], at: usize) -> Option<usize> {
    let b = p.get(at..at.checked_add(4)?)?;
    usize::try_from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok()
}

/// Indexes a comment payload; it must parse exactly, with no bytes after the last field.
///
/// # Errors
/// [`NotEditable::Malformed`] saying what does not parse.
pub fn index(payload: &[u8]) -> std::result::Result<CommentIndex, NotEditable> {
    let short = |what: &str| NotEditable::malformed(format!("the Vorbis comment {what}"));
    let vendor_len = le32(payload, 0).ok_or_else(|| short("has no vendor length"))?;
    let vendor_end = 4_usize
        .checked_add(vendor_len)
        .filter(|e| *e <= payload.len())
        .ok_or_else(|| short("vendor string runs past the block"))?;
    let count = le32(payload, vendor_end).ok_or_else(|| short("has no field count"))?;
    let mut pos = vendor_end + 4;
    // Each field takes at least its 4-byte length, so the count is checked before allocating.
    if count > (payload.len() - pos) / 4 {
        return Err(short("declares more fields than the block holds"));
    }
    let mut fields = Vec::with_capacity(count);
    for i in 0..count {
        let len = le32(payload, pos).ok_or_else(|| short(&format!("field {i} has no length")))?;
        let start = pos + 4;
        let end = start
            .checked_add(len)
            .filter(|e| *e <= payload.len())
            .ok_or_else(|| short(&format!("field {i} runs past the block")))?;
        fields.push(start..end);
        pos = end;
    }
    if pos != payload.len() {
        return Err(short(&format!(
            "has {} bytes after its last field",
            payload.len() - pos
        )));
    }
    Ok(CommentIndex {
        vendor: 0..vendor_end,
        fields,
    })
}

/// What an edit of the comment did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditSummary {
    /// Existing fields whose value was replaced in place.
    pub replaced: u32,
    /// Fields appended after the last existing one.
    pub appended: u32,
    /// The new payload's length, bytes.
    pub comment_bytes: u32,
}

/// The rebuilt payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    /// The new payload.
    pub bytes: Vec<u8>,
    /// What changed.
    pub summary: EditSummary,
}

fn push_field(out: &mut Vec<u8>, parts: &[&[u8]]) -> std::result::Result<(), NotEditable> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    let len32 = u32::try_from(len).map_err(|_| NotEditable::TooLarge { bytes: len as u64 })?;
    out.extend_from_slice(&len32.to_le_bytes());
    for p in parts {
        out.extend_from_slice(p);
    }
    Ok(())
}

/// Applies `edits` to the comment `payload` (see the module documentation).
///
/// # Errors
/// [`NotEditable::Malformed`] when the payload does not parse, [`NotEditable::TooLarge`] when
/// the result would not fit a metadata block.
pub fn edit_comment(
    payload: &[u8],
    edits: &[VorbisEdit],
) -> std::result::Result<Edited, NotEditable> {
    let idx = index(payload)?;
    let mut replaced = 0_u32;
    let mut used = vec![false; edits.len()];
    let mut out = Vec::with_capacity(payload.len() + edits.iter().map(|e| e.value.len() + 64).sum::<usize>());
    out.extend_from_slice(&payload[idx.vendor.clone()]);
    out.extend_from_slice(&[0; 4]); // the count, set below
    for range in &idx.fields {
        let hit = CommentIndex::name(payload, range)
            .and_then(|name| edits.iter().position(|e| e.matches(name)).map(|i| (name, i)));
        match hit {
            Some((name, i)) => {
                used[i] = true;
                replaced += 1;
                push_field(&mut out, &[name, b"=", edits[i].value.as_bytes()])?;
            }
            None => out.extend_from_slice(&payload[range.start - 4..range.end]),
        }
    }
    let mut appended = 0_u32;
    for (edit, _) in edits.iter().zip(&used).filter(|(_, u)| !**u) {
        push_field(&mut out, &[edit.name.as_bytes(), b"=", edit.value.as_bytes()])?;
        appended += 1;
    }
    let count = u32::try_from(idx.fields.len()).unwrap_or(u32::MAX).saturating_add(appended);
    let at = idx.vendor.end;
    out[at..at + 4].copy_from_slice(&count.to_le_bytes());
    let comment_bytes = u32::try_from(out.len())
        .ok()
        .filter(|l| *l <= MAX_BLOCK_BYTES)
        .ok_or(NotEditable::TooLarge {
            bytes: out.len() as u64,
        })?;
    Ok(Edited {
        bytes: out,
        summary: EditSummary {
            replaced,
            appended,
            comment_bytes,
        },
    })
}

#[cfg(test)]
mod tests;
