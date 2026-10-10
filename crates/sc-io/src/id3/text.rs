//! Text in `ID3v2` frames: reading the head of a `TXXX`, `COMM` or `GEOB` frame and a `TXXX`
//! value (ID3v2.3.0 sections 3.3, 4.2.2, 4.11 and 4.16; ID3v2.4.0 Main Structure section 4 and
//! Native Frames sections 4.2.6, 4.10 and 4.15) and writing SoundCheck's text frames
//! ([`Edit`]).

use sc_core::{Error, Result, TagEdit};

use super::MAX_VALUE_BYTES;
use super::parse::{FrameRef, HEADER_BYTES, encode_syncsafe};

/// Longest `TXXX` description of an edit, bytes of UTF-8.
const MAX_DESCRIPTION_BYTES: usize = 256;

/// The content of a frame body before its text: the extra bytes the format flags announce
/// skipped and v2.4 frame-level unsynchronisation undone; `None` for a compressed or
/// encrypted frame, whose content cannot be read without decompressing or decrypting it.
fn content(major: u8, flags: [u8; 2], body: &[u8]) -> Option<std::borrow::Cow<'_, [u8]>> {
    let format = flags[1];
    if major == 3 {
        // v2.3: 0x80 compression, 0x40 encryption, 0x20 grouping (one group byte).
        if format & 0xC0 != 0 {
            return None;
        }
        return body.get(usize::from(format & 0x20 != 0)..).map(Into::into);
    }
    // v2.4: 0x40 grouping (one byte), 0x08 compression, 0x04 encryption, 0x02
    // unsynchronisation, 0x01 data-length indicator (four bytes). Unsynchronisation covers
    // everything after the frame header, the group byte and indicator included, so it is
    // undone first.
    if format & 0x0C != 0 {
        return None;
    }
    let skip = usize::from(format & 0x40 != 0) + 4 * usize::from(format & 0x01 != 0);
    if format & 0x02 == 0 {
        return body.get(skip..).map(Into::into);
    }
    let mut out = Vec::with_capacity(body.len());
    let mut after_ff = false;
    for &b in body {
        if !(after_ff && b == 0) {
            out.push(b);
        }
        after_ff = b == 0xFF;
    }
    (skip <= out.len()).then(|| out.split_off(skip).into())
}

/// Decodes text in `encoding` up to its terminator (or the end): 0 ISO-8859-1, 1 UTF-16 with
/// a byte-order mark (little-endian without one), 2 UTF-16BE, 3 UTF-8. `None` for another
/// encoding byte.
pub(crate) fn decode_text(encoding: u8, b: &[u8]) -> Option<String> {
    match encoding {
        0 => Some(
            b.iter()
                .take_while(|c| **c != 0)
                .map(|c| char::from(*c))
                .collect(),
        ),
        3 => {
            let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            Some(String::from_utf8_lossy(&b[..n]).into_owned())
        }
        1 | 2 => {
            let mut big = encoding == 2;
            let mut units = b.as_chunks::<2>().0.iter().copied();
            let mut text = Vec::new();
            let mut first = true;
            for pair in units.by_ref() {
                if pair == [0, 0] {
                    break;
                }
                if first && encoding == 1 && (pair == [0xFF, 0xFE] || pair == [0xFE, 0xFF]) {
                    big = pair == [0xFE, 0xFF];
                } else {
                    text.push(if big {
                        u16::from_be_bytes(pair)
                    } else {
                        u16::from_le_bytes(pair)
                    });
                }
                first = false;
            }
            Some(String::from_utf16_lossy(&text))
        }
        _ => None,
    }
}

/// Splits `b` at the terminator of text in `encoding` (one zero byte for ISO-8859-1 and UTF-8,
/// an aligned zero pair for UTF-16): the text, for UTF-16 without a byte-order mark also its
/// big-endian reading, and the bytes after the terminator (none when it is missing). `None`
/// for an unknown encoding byte.
fn split_text(encoding: u8, b: &[u8]) -> Option<(String, Option<String>, &[u8])> {
    let (end, next) = if matches!(encoding, 1 | 2) {
        let pair = b.as_chunks::<2>().0.iter().position(|p| *p == [0, 0]);
        pair.map_or((b.len(), b.len()), |k| (2 * k, 2 * k + 2))
    } else {
        let zero = b.iter().position(|c| *c == 0);
        zero.map_or((b.len(), b.len()), |n| (n, n + 1))
    };
    let text = decode_text(encoding, &b[..end])?;
    let bom = b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]);
    let big_endian = (encoding == 1 && !bom)
        .then(|| decode_text(2, &b[..end]))
        .flatten();
    Some((text, big_endian, &b[next..]))
}

/// The head of a `TXXX`, `COMM` or `GEOB` frame: what comes before its value or object.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FrameHead {
    /// The text encoding byte, 0-3.
    pub encoding: u8,
    /// The description (`TXXX`), short content description (`COMM`) or content description
    /// (`GEOB`); UTF-16 without a byte-order mark read little-endian.
    pub description: String,
    /// The description read big-endian, for UTF-16 without a byte-order mark.
    pub description_be: Option<String>,
    /// `COMM`: the ISO 639-2 language code as stored (three bytes).
    pub language: Option<[u8; 3]>,
    /// `GEOB`: the MIME type (always ISO-8859-1).
    pub mime: Option<String>,
    /// `GEOB`: the file name.
    pub file_name: Option<String>,
}

/// The head of a `TXXX`, `COMM` or `GEOB` frame with these flags and body (ID3v2.3.0
/// sections 4.2.2, 4.11 and 4.16; ID3v2.4.0 Native Frames 4.2.6, 4.10 and 4.15), when it can
/// be read: `None` for another frame id, a compressed or encrypted frame, an unknown text
/// encoding, or a `COMM`/`GEOB` body too short for its encoding byte (and language). Strings
/// without their terminator end with the body. An empty `TXXX` body reads as an empty
/// description.
pub(crate) fn frame_head(major: u8, id: [u8; 4], flags: [u8; 2], body: &[u8]) -> Option<FrameHead> {
    if !matches!(&id, b"TXXX" | b"COMM" | b"GEOB") {
        return None;
    }
    let content = content(major, flags, body)?;
    let Some((&encoding, rest)) = content.split_first() else {
        return (&id == b"TXXX").then(FrameHead::default);
    };
    let mut head = FrameHead {
        encoding,
        ..FrameHead::default()
    };
    let described = match &id {
        b"COMM" => {
            let (lang, rest) = rest.split_first_chunk::<3>()?;
            head.language = Some(*lang);
            rest
        }
        b"GEOB" => {
            let (mime, _, rest) = split_text(0, rest)?;
            let (file_name, _, rest) = split_text(encoding, rest)?;
            head.mime = Some(mime);
            head.file_name = Some(file_name);
            rest
        }
        _ => rest,
    };
    let (description, description_be, _) = split_text(encoding, described)?;
    head.description = description;
    head.description_be = description_be;
    Some(head)
}

/// The description of a `TXXX` frame with these flags and body, when it can be read, and for
/// UTF-16 without a byte-order mark also its big-endian reading. An empty body reads as an
/// empty description. (The tests' view of [`frame_head`].)
#[cfg(test)]
pub(crate) fn txxx_descriptions(
    major: u8,
    flags: [u8; 2],
    body: &[u8],
) -> Option<(String, Option<String>)> {
    frame_head(major, *b"TXXX", flags, body).map(|h| (h.description, h.description_be))
}

/// The value of a `TXXX` frame with these flags and body: the text after the description, up
/// to its terminator or the end. `None` when the frame cannot be read.
pub(crate) fn txxx_value(major: u8, flags: [u8; 2], body: &[u8]) -> Option<String> {
    let content = content(major, flags, body)?;
    let (&encoding, rest) = content.split_first()?;
    let (_, _, value) = split_text(encoding, rest)?;
    decode_text(encoding, value)
}

/// What an edit writes: a text frame by id, or a `TXXX` frame by description.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Label {
    /// A text information frame (`TBPM`, `TKEY`, ...): four characters `A-Z`/`0-9` starting
    /// with `T`, not `TXXX`.
    Frame([u8; 4]),
    /// A user text frame `TXXX` with this description (compared ASCII case-insensitively).
    Txxx(String),
}

/// One text frame SoundCheck writes: a label and its value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Edit {
    label: Label,
    value: String,
}

fn invalid(what: &str) -> Error {
    Error::InvalidArgument(format!("tag edit: {what}"))
}

impl Edit {
    /// An edit from a label (`TBPM`, `TXXX:SOUNDCHECK`) and a value.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for another kind of label, an empty or overlong `TXXX`
    /// description (over 256 bytes), a NUL character, or a value over
    /// [`MAX_VALUE_BYTES`].
    pub fn new(label: &str, value: &str) -> Result<Self> {
        if label.contains('\0') || value.contains('\0') {
            return Err(invalid(&format!("{label:?} holds a NUL character")));
        }
        if value.len() > MAX_VALUE_BYTES {
            return Err(invalid(&format!(
                "the value of {label:?} has {} bytes, at most {MAX_VALUE_BYTES}",
                value.len()
            )));
        }
        let label = if let Some(desc) = label.strip_prefix("TXXX:") {
            if desc.is_empty() || desc.len() > MAX_DESCRIPTION_BYTES {
                return Err(invalid(&format!(
                    "a TXXX description has 1 to {MAX_DESCRIPTION_BYTES} bytes, not {}",
                    desc.len()
                )));
            }
            Label::Txxx(desc.to_string())
        } else {
            let id: [u8; 4] = label
                .as_bytes()
                .try_into()
                .map_err(|_| invalid(&format!("{label:?} is not a frame id or TXXX:<name>")))?;
            let valid = id[0] == b'T'
                && &id != b"TXXX"
                && id
                    .iter()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
            if !valid {
                return Err(invalid(&format!("{label:?} is not a text frame id")));
            }
            Label::Frame(id)
        };
        Ok(Self {
            label,
            value: value.to_string(),
        })
    }

    /// The label.
    #[must_use]
    pub fn label(&self) -> &Label {
        &self.label
    }

    /// The value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether this edit replaces `frame`: the same id, and for `TXXX` the same description
    /// (ASCII case-insensitive; UTF-16 without a byte-order mark in either byte order).
    #[must_use]
    pub fn matches(&self, frame: &FrameRef) -> bool {
        match &self.label {
            Label::Frame(id) => frame.id == *id,
            Label::Txxx(desc) => frame.is(b"TXXX", desc),
        }
    }

    fn same_label(&self, other: &Self) -> bool {
        match (&self.label, &other.label) {
            (Label::Frame(a), Label::Frame(b)) => a == b,
            (Label::Txxx(a), Label::Txxx(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        }
    }

    /// The whole frame (header and body) in a tag of major version `major` (3 or 4): flags 0,
    /// encoding 0 when every character is ASCII, else 1 (UTF-16 little-endian with a
    /// byte-order mark per string) in v2.3 and 3 (UTF-8) in v2.4; the `TXXX` description is
    /// terminated, the value is not.
    ///
    /// # Panics
    /// Never: [`Edit::new`] bounds the value and description, so the body stays far below
    /// the 28-bit frame size limit.
    #[must_use]
    pub fn frame(&self, major: u8) -> Vec<u8> {
        let (id, desc) = match &self.label {
            Label::Frame(id) => (*id, None),
            Label::Txxx(d) => (*b"TXXX", Some(d.as_str())),
        };
        let ascii = self.value.is_ascii() && desc.is_none_or(str::is_ascii);
        let mut body = Vec::with_capacity(8 + 2 * (self.value.len() + desc.map_or(0, str::len)));
        if ascii || major == 4 {
            body.push(if ascii { 0 } else { 3 });
            if let Some(d) = desc {
                body.extend_from_slice(d.as_bytes());
                body.push(0);
            }
            body.extend_from_slice(self.value.as_bytes());
        } else {
            body.push(1);
            let utf16 = |s: &str, out: &mut Vec<u8>| {
                out.extend_from_slice(&[0xFF, 0xFE]);
                out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
            };
            if let Some(d) = desc {
                utf16(d, &mut body);
                body.extend_from_slice(&[0, 0]);
            }
            utf16(&self.value, &mut body);
        }
        // The body is at most a few hundred KiB (value and description are bounded), far below
        // both the 32-bit and the 28-bit syncsafe limits.
        let len = u32::try_from(body.len()).expect("edit values are bounded");
        let size = if major == 3 {
            len.to_be_bytes()
        } else {
            encode_syncsafe(len).expect("edit values are bounded")
        };
        let mut frame = Vec::with_capacity(HEADER_BYTES + body.len());
        frame.extend_from_slice(&id);
        frame.extend_from_slice(&size);
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&body);
        frame
    }
}

/// Validated edits from a render request.
///
/// # Errors
/// [`Error::InvalidArgument`] for an invalid edit (see [`Edit::new`]) or two edits with the
/// same label.
pub fn edits_from(edits: &[TagEdit]) -> Result<Vec<Edit>> {
    let mut out: Vec<Edit> = Vec::with_capacity(edits.len());
    for e in edits {
        let edit = Edit::new(&e.label, &e.value)?;
        if out.iter().any(|o| o.same_label(&edit)) {
            return Err(invalid(&format!("{:?} is edited twice", e.label)));
        }
        out.push(edit);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
