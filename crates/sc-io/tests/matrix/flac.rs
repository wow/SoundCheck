//! A minimal FLAC writer for fixtures (RFC 9639), independent of any encoder: `fLaC`,
//! STREAMINFO with exact block/frame sizes, total samples and the MD5 of the PCM, the given
//! metadata blocks in order (last-block flag on the final one), then fixed-blocksize frames of
//! 4096 samples (the last one shorter) whose subframes are VERBATIM, channels coded
//! independently (no mid/side), each frame with its CRC-8 header check and CRC-16 footer.
//!
//! The CRCs here are bitwise; the inspector's are table-driven.

use md5::{Digest, Md5};

use super::parse::{Kind, Listed, sha256_hex};
use super::pcm::Samples;
use super::riff::Built;

/// Samples per frame (the last frame holds the remainder).
pub const BLOCK_SIZE: usize = 4096;

/// A metadata block after STREAMINFO.
#[derive(Debug, Clone)]
pub enum Meta {
    /// SEEKTABLE with a point at every `every`-th frame, then `placeholders` placeholder points.
    SeekTable {
        /// Frame spacing of the real points.
        every: usize,
        /// Placeholder points (sample number `u64::MAX`) at the end.
        placeholders: usize,
    },
    /// `VORBIS_COMMENT` with a vendor string and `NAME=value` fields in order.
    Vorbis {
        /// Vendor string.
        vendor: String,
        /// Fields as written.
        fields: Vec<String>,
    },
    /// PICTURE of type 3 (front cover).
    Picture {
        /// MIME type.
        mime: String,
        /// Image bytes.
        data: Vec<u8>,
    },
    /// APPLICATION with a 4-byte id.
    Application {
        /// Registered application id.
        id: [u8; 4],
        /// Opaque data.
        data: Vec<u8>,
    },
    /// CUESHEET as [`cuesheet`] writes it.
    Cuesheet,
    /// PADDING of this many zero bytes.
    Padding(usize),
    /// A block of a reserved type, carried opaquely.
    Unknown {
        /// Block type, 7..=126.
        ty: u8,
        /// Opaque data.
        data: Vec<u8>,
    },
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for b in bytes {
        crc ^= u16::from(*b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The "UTF-8" coding of a frame number (RFC 9639 section 9.1.5).
#[must_use]
pub fn coded_number(n: u64) -> Vec<u8> {
    if n < 0x80 {
        return vec![u8::try_from(n).expect("below 0x80")];
    }
    let mut tail = Vec::new();
    let mut v = n;
    loop {
        tail.push(0x80 | u8::try_from(v & 0x3F).expect("6 bits"));
        v >>= 6;
        // With t continuation bytes the lead byte holds 6 - t payload bits.
        if v < 1 << (6 - tail.len()) {
            break;
        }
    }
    let len = tail.len() + 1;
    let lead_mask = !(0xFF_u8 >> len);
    let mut out = vec![lead_mask | u8::try_from(v).expect("fits in the lead byte")];
    out.extend(tail.iter().rev());
    out
}

/// One frame holding `block` (interleaved) as VERBATIM subframes.
fn frame(number: u64, block: &[i32], channels: u16, bits: u8, sample_rate: u32) -> Vec<u8> {
    let block_size = block.len() / usize::from(channels);
    let bs_code: u8 = match block_size {
        BLOCK_SIZE => 12,
        1..=256 => 6,
        _ => 7,
    };
    let rate_code: u8 = match sample_rate {
        44_100 => 9,
        48_000 => 10,
        _ => 0, // from STREAMINFO
    };
    let depth_code: u8 = if bits == 16 { 4 } else { 6 };
    let ch_code = u8::try_from(channels - 1).expect("1 or 2 channels");
    let mut f = vec![
        0xFF,
        0xF8,
        bs_code << 4 | rate_code,
        ch_code << 4 | depth_code << 1,
    ];
    f.extend(coded_number(number));
    if bs_code == 6 {
        f.push(u8::try_from(block_size - 1).expect("at most 256"));
    } else if bs_code == 7 {
        f.extend_from_slice(
            &u16::try_from(block_size - 1)
                .expect("below 65536")
                .to_be_bytes(),
        );
    }
    f.push(crc8(&f));
    let width = usize::from(bits / 8);
    for c in 0..usize::from(channels) {
        f.push(0x02); // zero bit, type 000001 (VERBATIM), no wasted bits
        for s in block.iter().skip(c).step_by(usize::from(channels)) {
            f.extend_from_slice(&s.to_be_bytes()[4 - width..]);
        }
    }
    let crc = crc16(&f);
    f.extend_from_slice(&crc.to_be_bytes());
    f
}

/// CUESHEET (not CD-DA): track 1 at sample 0 (index 1 at 0), track 2 at sample 2205 (index 0
/// at 0, index 1 at 588, both relative to the track), lead-out track 255 at `total_samples`.
#[must_use]
pub fn cuesheet(total_samples: u64) -> Vec<u8> {
    let mut p = vec![0_u8; 128]; // media catalog number: none
    p.extend_from_slice(&0_u64.to_be_bytes()); // lead-in samples
    p.push(0); // not CD-DA
    p.extend(std::iter::repeat_n(0_u8, 258));
    p.push(3); // tracks
    let mut track = |offset: u64, number: u8, indices: &[(u64, u8)]| {
        p.extend_from_slice(&offset.to_be_bytes());
        p.push(number);
        p.extend_from_slice(&[0_u8; 12]); // ISRC: none
        p.extend_from_slice(&[0_u8; 14]); // audio track, no pre-emphasis, reserved
        p.push(u8::try_from(indices.len()).expect("few indices"));
        for (index_offset, index) in indices {
            p.extend_from_slice(&index_offset.to_be_bytes());
            p.push(*index);
            p.extend_from_slice(&[0_u8; 3]);
        }
    };
    track(0, 1, &[(0, 1)]);
    track(2205, 2, &[(0, 0), (588, 1)]);
    track(total_samples, 255, &[]);
    p
}

fn picture(mime: &str, data: &[u8]) -> Vec<u8> {
    let mut p = 3_u32.to_be_bytes().to_vec();
    let len = |n: usize| u32::try_from(n).expect("small picture").to_be_bytes();
    p.extend_from_slice(&len(mime.len()));
    p.extend_from_slice(mime.as_bytes());
    p.extend_from_slice(&len(0)); // description
    for v in [1_u32, 1, 24, 0] {
        p.extend_from_slice(&v.to_be_bytes()); // width, height, depth, colours
    }
    p.extend_from_slice(&len(data.len()));
    p.extend_from_slice(data);
    p
}

/// A `VORBIS_COMMENT` payload and the listing of its vendor and fields.
#[must_use]
pub fn vorbis(vendor: &str, fields: &[String]) -> (Vec<u8>, Vec<Listed>) {
    let mut nested = Vec::new();
    let len = |n: usize| u32::try_from(n).expect("short comments").to_le_bytes();
    let mut p = len(vendor.len()).to_vec();
    p.extend_from_slice(vendor.as_bytes());
    nested.push(Listed {
        kind: Kind::VorbisVendor,
        id: "vendor".into(),
        sha256: sha256_hex(vendor.as_bytes()),
        pad: None,
    });
    p.extend_from_slice(&len(fields.len()));
    for field in fields {
        p.extend_from_slice(&len(field.len()));
        p.extend_from_slice(field.as_bytes());
        nested.push(Listed {
            kind: Kind::VorbisField,
            id: field.split('=').next().unwrap_or_default().into(),
            sha256: sha256_hex(field.as_bytes()),
            pad: None,
        });
    }
    (p, nested)
}

/// Encoded audio frames.
#[derive(Debug, Clone)]
pub struct Encoded {
    /// All frames back to back.
    pub audio: Vec<u8>,
    /// Size of each frame, bytes.
    pub sizes: Vec<usize>,
    /// Offset of each frame from the first, bytes.
    pub offsets: Vec<u64>,
    /// Samples per channel.
    pub total: u64,
}

/// Encodes interleaved `bits`-bit samples into VERBATIM frames of [`BLOCK_SIZE`].
#[must_use]
pub fn encode(data: &[i32], bits: u8, channels: u16, sample_rate: u32) -> Encoded {
    let ch = usize::from(channels);
    let mut enc = Encoded {
        audio: Vec::new(),
        sizes: Vec::new(),
        offsets: Vec::new(),
        total: (data.len() / ch) as u64,
    };
    for (n, block) in (0_u64..).zip(data.chunks(BLOCK_SIZE * ch)) {
        let f = frame(n, block, channels, bits, sample_rate);
        enc.offsets.push(enc.audio.len() as u64);
        enc.audio.extend_from_slice(&f);
        enc.sizes.push(f.len());
    }
    enc
}

/// A STREAMINFO payload for `enc` (fixed block size, exact frame sizes, MD5 of `data`).
#[must_use]
pub fn streaminfo(enc: &Encoded, data: &[i32], bits: u8, channels: u16, rate: u32) -> Vec<u8> {
    let mut md5 = Md5::new();
    for s in data {
        md5.update(&s.to_le_bytes()[..usize::from(bits / 8)]);
    }
    let block16 = u16::try_from(BLOCK_SIZE).expect("4096 fits");
    let mut info = Vec::with_capacity(34);
    info.extend_from_slice(&block16.to_be_bytes());
    info.extend_from_slice(&block16.to_be_bytes());
    let size24 = |n: usize| u32::try_from(n).expect("small frames").to_be_bytes()[1..].to_vec();
    info.extend(size24(*enc.sizes.iter().min().expect("at least one frame")));
    info.extend(size24(*enc.sizes.iter().max().expect("at least one frame")));
    let packed = u64::from(rate) << 44
        | u64::from(channels - 1) << 41
        | u64::from(bits - 1) << 36
        | enc.total;
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&md5.finalize());
    info
}

/// A SEEKTABLE payload: a point at every `every`-th frame, then `placeholders` placeholders.
#[must_use]
pub fn seektable(enc: &Encoded, every: usize, placeholders: usize) -> Vec<u8> {
    let mut p = Vec::new();
    for (i, offset) in enc.offsets.iter().enumerate().step_by(every) {
        let first = u64::try_from(i * BLOCK_SIZE).expect("small fixtures");
        p.extend_from_slice(&first.to_be_bytes());
        p.extend_from_slice(&offset.to_be_bytes());
        let samples = (enc.total - first).min(BLOCK_SIZE as u64);
        p.extend_from_slice(&u16::try_from(samples).expect("<= 4096").to_be_bytes());
    }
    for _ in 0..placeholders {
        p.extend_from_slice(&u64::MAX.to_be_bytes());
        p.extend_from_slice(&[0_u8; 10]);
    }
    p
}

/// `fLaC`, the metadata blocks (type, payload) with the last-block flag on the final one, the
/// audio, wrapped by `leading` and `trailing` bytes.
#[must_use]
pub fn assemble(
    leading: &[u8],
    blocks: &[(u8, Vec<u8>)],
    audio: &[u8],
    trailing: &[u8],
) -> Vec<u8> {
    let mut bytes = leading.to_vec();
    bytes.extend_from_slice(b"fLaC");
    for (i, (ty, payload)) in blocks.iter().enumerate() {
        let flag = if i + 1 == blocks.len() { 0x80 } else { 0 };
        bytes.push(flag | ty);
        let len = u32::try_from(payload.len()).expect("blocks below 16 MiB");
        bytes.extend_from_slice(&len.to_be_bytes()[1..]);
        bytes.extend_from_slice(payload);
    }
    bytes.extend_from_slice(audio);
    bytes.extend_from_slice(trailing);
    bytes
}

/// Bytes around the stream: an `ID3v2` tag in front, an `ID3v1` tag behind.
#[derive(Debug, Clone, Default)]
pub struct Wrap {
    /// Bytes before `fLaC`.
    pub leading: Vec<u8>,
    /// Bytes after the last frame.
    pub trailing: Vec<u8>,
}

fn blob(kind: Kind, id: &str, bytes: &[u8]) -> Listed {
    Listed {
        kind,
        id: id.into(),
        sha256: sha256_hex(bytes),
        pad: None,
    }
}

/// Writes a FLAC stream of integer `samples` with the metadata blocks in `metas` after
/// STREAMINFO.
///
/// # Panics
/// For float samples (FLAC holds integers only).
#[must_use]
pub fn build(
    samples: &Samples,
    sample_rate: u32,
    channels: u16,
    metas: &[Meta],
    wrap: &Wrap,
) -> Built {
    let Samples::Int { bits, data } = samples else {
        panic!("FLAC fixtures hold integer PCM");
    };
    let enc = encode(data, *bits, channels, sample_rate);
    let mut blocks = vec![(0_u8, streaminfo(&enc, data, *bits, channels, sample_rate))];
    let mut listing = Vec::new();
    if !wrap.leading.is_empty() {
        listing.push(blob(Kind::LeadingTag, "id3v2", &wrap.leading));
    }
    listing.push(blob(Kind::FlacBlock, "STREAMINFO", &blocks[0].1));
    for meta in metas {
        let mut nested = Vec::new();
        let (ty, name, payload) = match meta {
            Meta::SeekTable {
                every,
                placeholders,
            } => (
                3,
                "SEEKTABLE".to_string(),
                seektable(&enc, *every, *placeholders),
            ),
            Meta::Vorbis { vendor, fields } => {
                let (p, n) = vorbis(vendor, fields);
                nested = n;
                (4, "VORBIS_COMMENT".into(), p)
            }
            Meta::Picture { mime, data } => (6, "PICTURE".into(), picture(mime, data)),
            Meta::Application { id, data } => {
                let mut p = id.to_vec();
                p.extend_from_slice(data);
                let name = format!("APPLICATION:{}", String::from_utf8_lossy(id));
                (2, name, p)
            }
            Meta::Cuesheet => (5, "CUESHEET".into(), cuesheet(enc.total)),
            Meta::Padding(n) => (1, "PADDING".into(), vec![0; *n]),
            Meta::Unknown { ty, data } => (*ty, format!("TYPE-{ty}"), data.clone()),
        };
        listing.push(blob(Kind::FlacBlock, &name, &payload));
        listing.extend(nested);
        blocks.push((ty, payload));
    }
    listing.push(blob(Kind::FlacFrames, "frames", &enc.audio));
    if !wrap.trailing.is_empty() {
        listing.push(blob(Kind::Trailing, "trailing", &wrap.trailing));
    }
    Built {
        bytes: assemble(&wrap.leading, &blocks, &enc.audio, &wrap.trailing),
        listing,
    }
}
