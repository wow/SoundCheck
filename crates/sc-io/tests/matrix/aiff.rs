//! AIFF and AIFF-C fixture builder (Apple "Audio Interchange File Format" 1.3, 1989, and the
//! AIFF-C draft of 1991): `FORM` with `COMM` (80-bit IEEE 754 extended sample rate), `SSND`
//! (offset 0, block size 0), `FVER`, `MARK`, text chunks, `APPL` and any other chunk, each
//! followed by a pad byte when its length is odd. Sizes are big-endian.
//!
//! Layout options reproduce the trailing-byte faults seen in DJ libraries exported by common
//! taggers, all after the last chunk: stray bytes inside the `FORM` (its size counts them),
//! bytes after the `FORM` end, and an odd last chunk without its pad byte, which the `FORM`
//! size then leaves out too.

use super::parse::{Kind, Listed, sha256_hex};
use super::riff::{Built, Chunk};

/// Form type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `AIFF`.
    Aiff,
    /// `AIFC`.
    Aifc,
}

/// Exact 80-bit extended encoding of an integer sample rate: sign 0, exponent `16383 + e`
/// where `2^e <= rate < 2^(e+1)`, mantissa `rate << (63 - e)` with the explicit integer bit.
///
/// # Panics
/// For a rate of 0, which has no normalised encoding.
#[must_use]
pub fn extended(rate: u32) -> [u8; 10] {
    assert!(rate > 0, "a sample rate is positive");
    let e = rate.ilog2();
    let exponent = u16::try_from(16383 + e).expect("exponent below 16383 + 32");
    let mantissa = u64::from(rate) << (63 - e);
    let mut out = [0_u8; 10];
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// A Pascal string (count byte + bytes) padded so its total length is even.
fn pstring(text: &str) -> Vec<u8> {
    let count = u8::try_from(text.len()).expect("pstrings hold at most 255 bytes");
    let mut v = vec![count];
    v.extend_from_slice(text.as_bytes());
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// A `COMM` chunk; `aifc` adds the compression type and its name (AIFF-C).
#[must_use]
pub fn comm(
    channels: u16,
    frames: u32,
    bits: u16,
    sample_rate: u32,
    aifc: Option<(&[u8; 4], &str)>,
) -> Chunk {
    let mut p = Vec::new();
    p.extend_from_slice(&channels.to_be_bytes());
    p.extend_from_slice(&frames.to_be_bytes());
    p.extend_from_slice(&bits.to_be_bytes());
    p.extend_from_slice(&extended(sample_rate));
    if let Some((compression, name)) = aifc {
        p.extend_from_slice(compression);
        p.extend(pstring(name));
    }
    Chunk::new(*b"COMM", p)
}

/// An `SSND` chunk with offset 0 and block size 0 around already-encoded sample bytes.
#[must_use]
pub fn ssnd(sample_bytes: &[u8]) -> Chunk {
    ssnd_with_offset(sample_bytes, 0)
}

/// An `SSND` chunk whose sound data starts `offset` bytes after the header (block size 0);
/// the skipped bytes hold a recognisable filler.
#[must_use]
pub fn ssnd_with_offset(sample_bytes: &[u8], offset: u32) -> Chunk {
    let mut p = offset.to_be_bytes().to_vec();
    p.extend_from_slice(&0_u32.to_be_bytes());
    p.extend((0..offset).map(|i| 0xA0 | u8::try_from(i & 0x0F).expect("4 bits")));
    p.extend_from_slice(sample_bytes);
    Chunk::new(*b"SSND", p)
}

/// A `CoreAudio` `CHAN` chunk: stereo layout tag (101 << 16 | 2), no bitmap, no descriptions.
#[must_use]
pub fn chan_stereo() -> Chunk {
    let mut p = 0x0065_0002_u32.to_be_bytes().to_vec();
    p.extend_from_slice(&0_u32.to_be_bytes());
    p.extend_from_slice(&0_u32.to_be_bytes());
    Chunk::new(*b"CHAN", p)
}

/// A `COMT` chunk with one comment (timestamp, marker id 0 = not linked, text).
#[must_use]
pub fn comt(text: &str) -> Chunk {
    let mut p = 1_u16.to_be_bytes().to_vec();
    p.extend_from_slice(&0xE0E0_0000_u32.to_be_bytes()); // timestamp, seconds since 1904
    p.extend_from_slice(&0_i16.to_be_bytes());
    let len = u16::try_from(text.len()).expect("short comment");
    p.extend_from_slice(&len.to_be_bytes());
    p.extend_from_slice(text.as_bytes());
    if text.len() % 2 == 1 {
        p.push(0);
    }
    Chunk::new(*b"COMT", p)
}

/// An `INST` chunk (base note 60, full key and velocity range, no loops).
#[must_use]
pub fn inst() -> Chunk {
    let mut p = vec![60, 0, 0, 127, 1, 127]; // note, detune, low/high note, low/high velocity
    p.extend_from_slice(&0_i16.to_be_bytes()); // gain, dB
    for _ in 0..2 {
        // sustain and release loop: play mode 0 (none), begin and end marker 0
        p.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    }
    Chunk::new(*b"INST", p)
}

/// The AIFF-C `FVER` chunk (version 1, timestamp 0xA2805140).
#[must_use]
pub fn fver() -> Chunk {
    Chunk::new(*b"FVER", 0xA280_5140_u32.to_be_bytes().to_vec())
}

/// A text chunk (`NAME`, `AUTH`, `ANNO`, `(c) `): bytes without terminator.
#[must_use]
pub fn text(id: [u8; 4], value: &str) -> Chunk {
    Chunk::new(id, value.as_bytes().to_vec())
}

/// An `APPL` chunk: application signature then opaque data.
#[must_use]
pub fn appl(signature: [u8; 4], data: &[u8]) -> Chunk {
    let mut p = signature.to_vec();
    p.extend_from_slice(data);
    Chunk::new(*b"APPL", p)
}

/// A `MARK` chunk from (id, position in frames, name).
#[must_use]
pub fn mark(markers: &[(u16, u32, &str)]) -> Chunk {
    let count = u16::try_from(markers.len()).expect("few markers");
    let mut p = count.to_be_bytes().to_vec();
    for (id, position, name) in markers {
        p.extend_from_slice(&id.to_be_bytes());
        p.extend_from_slice(&position.to_be_bytes());
        p.extend(pstring(name));
    }
    Chunk::new(*b"MARK", p)
}

/// Options for [`build_with`] (all off by default).
#[derive(Debug, Clone, Default)]
pub struct AiffLayout {
    /// Bytes after the last chunk inside the `FORM` (counted in its size).
    pub stray_inside: Vec<u8>,
    /// Bytes after the `FORM` end.
    pub trailing: Vec<u8>,
    /// Leave out the pad byte after an odd last chunk (and out of the `FORM` size).
    pub omit_last_pad: bool,
}

/// Writes a `FORM` file from chunks in order.
#[must_use]
pub fn build(form: Form, chunks: &[Chunk]) -> Built {
    build_with(form, chunks, &AiffLayout::default())
}

/// Writes a `FORM` file from chunks in order with the given layout. The bytes after the last
/// chunk, inside and after the `FORM`, are listed as one trailing blob, as a reader that
/// walks to the end of the file finds them.
#[must_use]
pub fn build_with(form: Form, chunks: &[Chunk], layout: &AiffLayout) -> Built {
    let mut body = Vec::new();
    let mut listing: Vec<Listed> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let last = i + 1 == chunks.len();
        let pad = chunk.payload.len() % 2 == 1 && !(last && layout.omit_last_pad);
        body.extend_from_slice(&chunk.id);
        let len = u32::try_from(chunk.payload.len()).expect("fixtures stay small");
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(&chunk.payload);
        if pad {
            body.push(chunk.pad);
        }
        listing.push(chunk.listed(pad));
        listing.extend(chunk.nested.iter().cloned());
    }
    body.extend_from_slice(&layout.stray_inside);
    let mut bytes = b"FORM".to_vec();
    let size = u32::try_from(4 + body.len()).expect("fixtures stay small");
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(match form {
        Form::Aiff => b"AIFF",
        Form::Aifc => b"AIFC",
    });
    bytes.extend_from_slice(&body);
    bytes.extend_from_slice(&layout.trailing);
    let after = [&layout.stray_inside[..], &layout.trailing[..]].concat();
    if !after.is_empty() {
        listing.push(Listed {
            kind: Kind::Trailing,
            id: "trailing".into(),
            sha256: sha256_hex(&after),
            pad: None,
        });
    }
    Built { bytes, listing }
}
