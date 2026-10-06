//! Editing an `ID3v2` tag without touching what SoundCheck does not own.
//!
//! Specifications: id3.org "ID3 tag version 2.3.0" (informal standard, 1999) and "ID3 tag
//! version 2.4.0 - Main Structure" plus "- Native Frames" (2000). A tag is a 10-byte header
//! (`ID3`, major version, revision, flags, a 28-bit syncsafe size), an optional extended header,
//! frames (a 10-byte header: four-character id, a size that is a plain big-endian 32-bit integer
//! in v2.3 and syncsafe in v2.4, two flag bytes) and zero padding; v2.4 may end with a 10-byte
//! footer (`3DI`) instead of padding.
//!
//! - [`parse_tag`] indexes a tag: header, extended header, and every frame's id, flags and byte
//!   range. Frame bodies are not decoded, except the description of a `TXXX` frame (text
//!   encodings 0-3: ISO-8859-1, UTF-16 with a byte-order mark (little-endian when it has none),
//!   UTF-16BE, UTF-8; up to its terminator), read after the frame's grouping byte and v2.4
//!   data-length indicator are skipped and v2.4 frame-level unsynchronisation is undone; UTF-16
//!   without a byte-order mark is matched in both byte orders. A compressed or encrypted
//!   frame's description cannot be read, so a `TXXX` edit on a tag holding one is not made
//!   (appending could duplicate it).
//! - [`edit_tag`] writes SoundCheck's text frames ([`Edit`]: `TBPM`, `TXXX:SOUNDCHECK`,
//!   `TXXX:REPLAYGAIN_TRACK_GAIN`, ...). A frame with the same label (the same id, or for `TXXX`
//!   the same description compared ASCII case-insensitively) is replaced in place, every one if
//!   there are several; the other edits are appended once after the last frame, in request
//!   order. Every other frame keeps its bytes and its place. The header keeps its version,
//!   revision and flags; an extended header is kept (in v2.3 its padding-size field is
//!   updated). The new frames go into the existing padding when they fit (the tag keeps its
//!   size); otherwise the tag grows and gets [`GROWTH_PADDING_BYTES`] of fresh padding so that a
//!   later edit fits in place. Padding is zero-filled. A v2.4 tag with a footer has no padding
//!   (the specification forbids it) and its footer is rewritten with the new size. Bytes after
//!   the tag inside the same container chunk are carried after it.
//! - Our frames have flags 0, a text encoding of ISO-8859-1 (0) when the description and value
//!   are ASCII, else UTF-16 with a byte-order mark (1, little-endian) in v2.3 and UTF-8 (3) in
//!   v2.4, and no terminator after the value.
//!
//! A tag is not edited, and is carried unchanged, when editing could lose or corrupt something
//! ([`NotEditable`]): a version other than 2.3 or 2.4, tag-level unsynchronisation, flags the
//! specification does not define, an extended header with a CRC (it would no longer match) or
//! with v2.4 tag restrictions (our frames could break them), a tag over [`MAX_TAG_BYTES`], or a
//! structure that does not parse (a frame running past the tag, an invalid frame id, non-zero
//! bytes in the padding, more than [`MAX_FRAMES`] frames, a footer that does not repeat the
//! header). A v2.4 tag whose last frame is not a text frame and also reads consistently with a
//! plain 32-bit size (as old iTunes versions wrote them), its zero tail then frame data rather
//! than padding, is refused as ambiguous: an edit would write into that tail. Nothing is allocated from a declared
//! size before it is checked against the bytes at hand.

mod edit;
mod parse;
mod text;

#[cfg(test)]
pub(crate) mod test_build;

pub use edit::{EditSummary, Edited, edit_tag};
pub use parse::{FrameRef, TagIndex, decode_syncsafe, encode_syncsafe, parse_tag};
pub use text::{Edit, Label, edits_from};

/// Largest tag (chunk payload) SoundCheck reads to edit, bytes; larger tags are carried.
pub const MAX_TAG_BYTES: u64 = 16 << 20;

/// Most frames a tag may hold to be edited.
pub const MAX_FRAMES: usize = 65_536;

/// Padding given to a tag that has to grow, bytes: room for a later edit in place.
pub const GROWTH_PADDING_BYTES: usize = 1024;

/// Largest value of one edit, bytes of UTF-8.
pub const MAX_VALUE_BYTES: usize = 64 << 10;

/// Why requested tag edits were not written; the tag (if any) is then carried unchanged.
/// The ID3 reasons apply to WAV/AIFF files; FLAC files use [`NotEditable::NoVorbisComment`],
/// [`NotEditable::SeveralVorbisComments`], [`NotEditable::TooLarge`] and
/// [`NotEditable::Malformed`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotEditable {
    /// The FLAC file holds no `VORBIS_COMMENT` block (none is created).
    NoVorbisComment,
    /// The FLAC file holds more than one `VORBIS_COMMENT` block (RFC 9639 allows one).
    SeveralVorbisComments {
        /// Number of blocks.
        count: usize,
    },
    /// The file holds no ID3 tag.
    NoTag,
    /// The file holds more than one ID3 tag; readers disagree on which one counts.
    SeveralTags {
        /// Number of ID3 chunks.
        count: usize,
    },
    /// A tag version other than 2.3 and 2.4.
    UnsupportedVersion {
        /// The major version byte.
        major: u8,
    },
    /// Tag-level unsynchronisation (header flag 0x80).
    Unsynchronised,
    /// Header flags the tag's version does not define.
    UnknownFlags {
        /// The header flags byte.
        flags: u8,
    },
    /// The extended header carries a CRC-32 of the frames, which an edit would invalidate.
    ExtendedHeaderCrc,
    /// The v2.4 extended header declares tag restrictions.
    Restricted,
    /// A `TXXX` edit is requested and a `TXXX` frame's description cannot be read (a
    /// compressed or encrypted frame, an unknown text encoding): it might hold the same label,
    /// and appending ours would duplicate it.
    UnreadableTxxx,
    /// The tag is larger than [`MAX_TAG_BYTES`] or would grow past what its size field holds.
    TooLarge {
        /// The tag's size, bytes.
        bytes: u64,
    },
    /// The tag does not parse.
    Malformed {
        /// What is wrong.
        detail: String,
    },
}

impl NotEditable {
    pub(crate) fn malformed(detail: impl Into<String>) -> Self {
        Self::Malformed {
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for NotEditable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoVorbisComment => write!(f, "the file has no Vorbis comment block"),
            Self::SeveralVorbisComments { count } => {
                write!(f, "the file has {count} Vorbis comment blocks")
            }
            Self::NoTag => write!(f, "the file has no ID3 tag"),
            Self::SeveralTags { count } => write!(f, "the file has {count} ID3 tags"),
            Self::UnsupportedVersion { major } => write!(f, "ID3v2.{major} tags are not edited"),
            Self::Unsynchronised => write!(f, "the tag is unsynchronised"),
            Self::UnknownFlags { flags } => {
                write!(f, "the tag header has undefined flags ({flags:#04x})")
            }
            Self::ExtendedHeaderCrc => write!(f, "the tag is protected by a CRC"),
            Self::Restricted => write!(f, "the tag declares restrictions"),
            Self::UnreadableTxxx => {
                write!(f, "a TXXX frame's description cannot be read")
            }
            Self::TooLarge { bytes } => write!(f, "the tag is too large to edit ({bytes} bytes)"),
            Self::Malformed { detail } => write!(f, "the tag is malformed: {detail}"),
        }
    }
}

#[cfg(test)]
mod tests;
