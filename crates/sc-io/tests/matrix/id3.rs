//! `ID3v2`.3 and `ID3v2`.4 tag builder (id3.org "ID3 tag version 2.3.0" and "ID3 tag version
//! 2.4.0 - Main Structure / Native Frames"): a 10-byte header with a syncsafe size, frames with
//! a 10-byte header (size plain 32-bit in v2.3, syncsafe in v2.4), optional zero padding, an
//! optional extended header, v2.4 frame format flags (data-length indicator 0x01,
//! unsynchronisation 0x02) and the tag-level unsynchronisation flag.
//!
//! The Serato frames follow the published layout of Holzhaus' "serato-tags" notes: GEOB
//! objects with MIME `application/octet-stream` and the descriptions `Serato Autotags`,
//! `Serato Markers2` and `Serato BeatGrid`. Their payloads are plausible but are only ever
//! carried opaquely, so their exact semantics do not matter here.

use super::parse::{Kind, Listed, sha256_hex};

/// Tag version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// `ID3v2`.3.0.
    V23,
    /// `ID3v2`.4.0.
    V24,
}

/// One frame: id, label as the parser derives it, body.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Four-character frame id.
    pub id: [u8; 4],
    /// `ID` or `ID:description` (see `parse.rs`).
    pub label: String,
    /// Frame body (after the 10-byte frame header), as stored.
    pub body: Vec<u8>,
    /// The two frame flag bytes (status, format).
    pub flags: [u8; 2],
}

/// A built tag and its frame listing.
#[derive(Debug, Clone)]
pub struct Tag {
    /// Header, frames and padding.
    pub bytes: Vec<u8>,
    /// One entry per frame, in order.
    pub frames: Vec<Listed>,
}

/// A 1x1 PNG (signature, IHDR, IDAT, IEND); cover art content is never decoded.
pub const PNG_1X1: [u8; 67] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// The 28-bit syncsafe encoding of `v`.
#[must_use]
pub fn syncsafe(v: usize) -> [u8; 4] {
    assert!(v < 1 << 28, "syncsafe sizes hold 28 bits");
    // Each byte is masked to 7 bits, so the narrowing is exact.
    #[allow(clippy::cast_possible_truncation)]
    [
        (v >> 21 & 0x7F) as u8,
        (v >> 14 & 0x7F) as u8,
        (v >> 7 & 0x7F) as u8,
        (v & 0x7F) as u8,
    ]
}

/// Text encoding byte: ISO-8859-1 (0) in v2.3, UTF-8 (3) in v2.4.
#[must_use]
pub fn encoding(version: Version) -> u8 {
    match version {
        Version::V23 => 0,
        Version::V24 => 3,
    }
}

fn frame(id: [u8; 4], desc: Option<&str>, body: Vec<u8>) -> Frame {
    let id_text = String::from_utf8_lossy(&id).into_owned();
    Frame {
        id,
        label: match desc {
            Some(d) if !d.is_empty() => format!("{id_text}:{d}"),
            _ => id_text,
        },
        body,
        flags: [0, 0],
    }
}

/// A text frame (`TIT2`, `TPE1`, `TBPM`, `TKEY`, ...).
#[must_use]
pub fn text(version: Version, id: [u8; 4], value: &str) -> Frame {
    let mut body = vec![encoding(version)];
    body.extend_from_slice(value.as_bytes());
    frame(id, None, body)
}

/// A text frame in UTF-16 with a little-endian byte-order mark (encoding 1, both versions).
#[must_use]
pub fn text_utf16(id: [u8; 4], value: &str) -> Frame {
    let mut body = vec![1, 0xFF, 0xFE];
    body.extend(value.encode_utf16().flat_map(u16::to_le_bytes));
    body.extend_from_slice(&[0, 0]);
    frame(id, None, body)
}

/// A `TXXX` user text frame.
#[must_use]
pub fn txxx(version: Version, desc: &str, value: &str) -> Frame {
    let mut body = vec![encoding(version)];
    body.extend_from_slice(desc.as_bytes());
    body.push(0);
    body.extend_from_slice(value.as_bytes());
    frame(*b"TXXX", Some(desc), body)
}

/// A `COMM` frame.
#[must_use]
pub fn comm(version: Version, desc: &str, value: &str) -> Frame {
    let mut body = vec![encoding(version)];
    body.extend_from_slice(b"eng");
    body.extend_from_slice(desc.as_bytes());
    body.push(0);
    body.extend_from_slice(value.as_bytes());
    frame(*b"COMM", Some(desc), body)
}

/// An `APIC` frame holding a front cover (type 3).
#[must_use]
pub fn apic(mime: &str, image: &[u8]) -> Frame {
    let mut body = vec![0];
    body.extend_from_slice(mime.as_bytes());
    body.push(0);
    body.push(3);
    body.push(0); // empty description
    body.extend_from_slice(image);
    frame(*b"APIC", None, body)
}

/// A `PRIV` frame.
#[must_use]
pub fn private(owner: &str, data: &[u8]) -> Frame {
    let mut body = owner.as_bytes().to_vec();
    body.push(0);
    body.extend_from_slice(data);
    frame(*b"PRIV", Some(owner), body)
}

/// A `GEOB` frame with MIME `application/octet-stream` and an empty file name.
#[must_use]
pub fn geob(desc: &str, data: &[u8]) -> Frame {
    let mut body = vec![0];
    body.extend_from_slice(b"application/octet-stream\0");
    body.push(0); // file name
    body.extend_from_slice(desc.as_bytes());
    body.push(0);
    body.extend_from_slice(data);
    frame(*b"GEOB", Some(desc), body)
}

/// Standard base64 (RFC 4648) with padding.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 0x3F) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `Serato Autotags` payload: version 0x01 0x01, then BPM, auto-gain and gain as NUL-ended text.
#[must_use]
pub fn serato_autotags(bpm: &str) -> Vec<u8> {
    let mut v = vec![0x01, 0x01];
    for field in [bpm, "-3.257", "0.000"] {
        v.extend_from_slice(field.as_bytes());
        v.push(0);
    }
    v
}

/// `Serato Markers2` payload: 0x01 0x01, base64 of the entry list (`COLOR`, one `CUE`,
/// `BPMLOCK`), zero-padded to 470 bytes as Serato writes it.
#[must_use]
pub fn serato_markers2(cue_ms: u32) -> Vec<u8> {
    let mut entries = vec![0x01, 0x01];
    let mut entry = |name: &str, data: &[u8]| {
        entries.extend_from_slice(name.as_bytes());
        entries.push(0);
        let len = u32::try_from(data.len()).expect("short entry");
        entries.extend_from_slice(&len.to_be_bytes());
        entries.extend_from_slice(data);
    };
    entry("COLOR", &[0x00, 0xFF, 0xFF, 0xFF]);
    let mut cue = vec![0x00, 0x00];
    cue.extend_from_slice(&cue_ms.to_be_bytes());
    cue.extend_from_slice(&[0x00, 0xCC, 0x00, 0x00, 0x00, 0x00]);
    cue.extend_from_slice(b"Drop\0");
    entry("CUE", &cue);
    entry("BPMLOCK", &[0x00]);
    entries.push(0);
    let mut v = vec![0x01, 0x01];
    v.extend_from_slice(base64(&entries).as_bytes());
    v.resize(v.len().max(470), 0);
    v
}

/// `Serato BeatGrid` payload: version 0x01 0x00, marker count (u32 BE), one terminal marker
/// (position in seconds and BPM, both f32 BE), one footer byte.
#[must_use]
pub fn serato_beatgrid(position_s: f32, bpm: f32) -> Vec<u8> {
    let mut v = vec![0x01, 0x00];
    v.extend_from_slice(&1_u32.to_be_bytes());
    v.extend_from_slice(&position_s.to_be_bytes());
    v.extend_from_slice(&bpm.to_be_bytes());
    v.push(0x00);
    v
}

/// The common frame set: title, artist (UTF-16 in v2.3), BPM, key, comment, `TXXX:SOURCE`,
/// cover, a Windows Media `PRIV`; with `serato` the three Serato GEOB objects (all longer than
/// 127 bytes, so v2.3 and v2.4 sizes differ); with `replaygain` an existing
/// `TXXX:REPLAYGAIN_TRACK_GAIN` written by another tool.
#[must_use]
pub fn frames(version: Version, serato: bool, replaygain: bool) -> Vec<Frame> {
    let mut v = vec![text(version, *b"TIT2", "Matrix Tone")];
    v.push(match version {
        Version::V23 => text_utf16(*b"TPE1", "SoundCheck Ensemble"),
        Version::V24 => text(version, *b"TPE1", "SoundCheck Ensemble"),
    });
    v.push(text(version, *b"TBPM", "128"));
    if replaygain {
        v.push(txxx(version, "REPLAYGAIN_TRACK_GAIN", "-4.10 dB"));
    }
    v.push(text(version, *b"TKEY", "8A"));
    v.push(comm(version, "", "carried byte for byte"));
    v.push(txxx(version, "SOURCE", "synthetic fixture"));
    v.push(apic("image/png", &PNG_1X1));
    v.push(private(
        "WM/MediaClassPrimaryID",
        &[
            0xBC, 0x7D, 0x60, 0xD1, 0x23, 0xE3, 0xE2, 0x4B, 0x86, 0xA1, 0x48, 0xA4, 0x2A, 0x28,
            0x44, 0x1E,
        ],
    ));
    if serato {
        v.push(geob("Serato Autotags", &serato_autotags("128.00")));
        v.push(geob("Serato Markers2", &serato_markers2(1_875)));
        v.push(geob("Serato BeatGrid", &serato_beatgrid(0.012, 128.0)));
    }
    v
}

/// `ID3v2`.4 unsynchronisation of frame data: a 0x00 after every 0xFF (decoders drop a 0x00
/// that follows 0xFF, so this is always reversible).
#[must_use]
pub fn unsynchronise(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    for &b in data {
        out.push(b);
        if b == 0xFF {
            out.push(0);
        }
    }
    out
}

/// A v2.4 frame stored with format flags: 0x01 adds a data-length indicator (syncsafe length of
/// the frame data before unsynchronisation), 0x02 unsynchronises the data.
#[must_use]
pub fn with_format_flags(mut frame: Frame, format: u8) -> Frame {
    let mut body = Vec::new();
    if format & 0x01 != 0 {
        body.extend_from_slice(&syncsafe(frame.body.len()));
    }
    if format & 0x02 != 0 {
        body.extend(unsynchronise(&frame.body));
    } else {
        body.extend_from_slice(&frame.body);
    }
    frame.body = body;
    frame.flags = [0, format];
    frame
}

/// A frame as stored in a tag of `version`: id, size, flags, body.
#[must_use]
pub fn frame_bytes(version: Version, f: &Frame) -> Vec<u8> {
    let mut out = f.id.to_vec();
    match version {
        Version::V23 => {
            let len = u32::try_from(f.body.len()).expect("small frames");
            out.extend_from_slice(&len.to_be_bytes());
        }
        Version::V24 => out.extend_from_slice(&syncsafe(f.body.len())),
    }
    out.extend_from_slice(&f.flags);
    out.extend_from_slice(&f.body);
    out
}

/// An extended header: v2.3 with a padding-size field, v2.4 with one empty flag byte.
#[must_use]
pub fn ext_header(version: Version, padding: usize) -> Vec<u8> {
    match version {
        Version::V23 => {
            let mut v = vec![0, 0, 0, 6, 0, 0];
            let pad = u32::try_from(padding).expect("small padding");
            v.extend_from_slice(&pad.to_be_bytes());
            v
        }
        Version::V24 => vec![0, 0, 0, 6, 1, 0],
    }
}

/// Header, extended header, frames and zero padding as one tag.
#[must_use]
pub fn assemble(head: [u8; 3], ext: &[u8], frames: &[u8], padding: usize) -> Vec<u8> {
    let size = ext.len() + frames.len() + padding;
    let mut out = b"ID3".to_vec();
    out.extend_from_slice(&head);
    out.extend_from_slice(&syncsafe(size));
    out.extend_from_slice(ext);
    out.extend_from_slice(frames);
    out.resize(out.len() + padding, 0);
    out
}

/// How a tag is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Tag version.
    pub version: Version,
    /// Zero padding after the frames, bytes.
    pub padding: usize,
    /// Write an extended header (sets header flag 0x40).
    pub ext: bool,
    /// Set the tag-level unsynchronisation flag 0x80 (v2.4: every frame must carry format flag
    /// 0x02 then).
    pub unsync: bool,
}

/// Writes a tag: header, frames, `padding` zero bytes.
#[must_use]
pub fn tag(version: Version, frames: &[Frame], padding: usize) -> Tag {
    tag_with(
        Layout {
            version,
            padding,
            ext: false,
            unsync: false,
        },
        frames,
    )
}

/// Writes a tag with the given layout.
#[must_use]
pub fn tag_with(layout: Layout, frames: &[Frame]) -> Tag {
    let mut body = Vec::new();
    let mut listing = Vec::new();
    let ext = if layout.ext {
        let ext = ext_header(layout.version, layout.padding);
        listing.push(Listed {
            kind: Kind::Id3ExtHeader,
            id: "ext-header".into(),
            sha256: sha256_hex(&ext),
            pad: None,
        });
        ext
    } else {
        Vec::new()
    };
    for f in frames {
        let bytes = frame_bytes(layout.version, f);
        listing.push(Listed {
            kind: Kind::Id3Frame,
            id: f.label.clone(),
            sha256: sha256_hex(&bytes),
            pad: None,
        });
        body.extend(bytes);
    }
    let major = match layout.version {
        Version::V23 => 3,
        Version::V24 => 4,
    };
    let flags = (u8::from(layout.ext) * 0x40) | (u8::from(layout.unsync) * 0x80);
    Tag {
        bytes: assemble([major, 0, flags], &ext, &body, layout.padding),
        frames: listing,
    }
}
