//! WAV and RF64 fixture builder (Microsoft/IBM RIFF 1991 with the WAVEFORMATEX and
//! WAVEFORMATEXTENSIBLE `fmt ` layouts, EBU Tech 3285 `bext` v0/v1/v2, EBU Tech 3306 RF64 `ds64`,
//! iXML 1.5, the RIFF `cue `/`smpl`/`LIST INFO` chunks).
//!
//! Chunks are written in the given order with a pad byte after every odd-length payload, except
//! where a layout option reproduces a quirk seen in real files: an odd `data` chunk without its
//! pad byte (the RIFF size then ends at the last sample) and stray bytes after the RIFF end.

use super::parse::{Kind, Listed, sha256_hex};
use super::pcm::{Samples, bytes_le};

/// A chunk to write: id, payload, and the blocks nested in it (ID3 frames) for the listing.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Four-character id.
    pub id: [u8; 4],
    /// Payload without header or pad byte.
    pub payload: Vec<u8>,
    /// Blocks inside the payload, listed after the chunk itself.
    pub nested: Vec<Listed>,
    /// Value of the pad byte written after an odd-length payload (0 unless a fixture
    /// reproduces a writer that pads with something else).
    pub pad: u8,
}

impl Chunk {
    /// A chunk without nested blocks.
    #[must_use]
    pub fn new(id: [u8; 4], payload: Vec<u8>) -> Self {
        Self {
            id,
            payload,
            nested: Vec::new(),
            pad: 0,
        }
    }

    /// The same chunk with `pad` as its pad byte.
    #[must_use]
    pub fn with_pad(mut self, pad: u8) -> Self {
        self.pad = pad;
        self
    }

    /// The listing entry of this chunk (`padded`: whether the pad byte was written).
    #[must_use]
    pub fn listed(&self, padded: bool) -> Listed {
        Listed {
            kind: Kind::Chunk,
            id: String::from_utf8_lossy(&self.id).into_owned(),
            sha256: sha256_hex(&self.payload),
            pad: padded.then_some(self.pad),
        }
    }
}

/// A built file and the blocks it contains, in order, as the builder wrote them.
#[derive(Debug, Clone)]
pub struct Built {
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Blocks in file order.
    pub listing: Vec<Listed>,
}

/// Layout of the `fmt ` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormat {
    /// `WAVE_FORMAT_PCM` (0x0001), 16-byte chunk.
    Pcm,
    /// `WAVE_FORMAT_EXTENSIBLE` (0xFFFE), 40-byte chunk, PCM sub-format, front L/R mask.
    Extensible,
    /// `WAVE_FORMAT_IEEE_FLOAT` (0x0003), 18-byte chunk with `cbSize` 0.
    Float,
}

/// `KSDATAFORMAT_SUBTYPE_PCM` after its first two bytes (00000001-0000-0010-8000-00AA00389B71).
const PCM_GUID_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// A `fmt ` chunk.
#[must_use]
pub fn fmt(format: WavFormat, channels: u16, sample_rate: u32, bits: u16) -> Chunk {
    let align = channels * bits / 8;
    let tag: u16 = match format {
        WavFormat::Pcm => 1,
        WavFormat::Extensible => 0xFFFE,
        WavFormat::Float => 3,
    };
    let mut p = Vec::new();
    p.extend_from_slice(&tag.to_le_bytes());
    p.extend_from_slice(&channels.to_le_bytes());
    p.extend_from_slice(&sample_rate.to_le_bytes());
    p.extend_from_slice(&(sample_rate * u32::from(align)).to_le_bytes());
    p.extend_from_slice(&align.to_le_bytes());
    p.extend_from_slice(&bits.to_le_bytes());
    match format {
        WavFormat::Pcm => {}
        WavFormat::Float => p.extend_from_slice(&0_u16.to_le_bytes()),
        WavFormat::Extensible => {
            p.extend_from_slice(&22_u16.to_le_bytes()); // cbSize
            p.extend_from_slice(&bits.to_le_bytes()); // wValidBitsPerSample
            let mask: u32 = if channels == 1 { 0x4 } else { 0x3 };
            p.extend_from_slice(&mask.to_le_bytes());
            p.extend_from_slice(&1_u16.to_le_bytes()); // sub-format: PCM
            p.extend_from_slice(&PCM_GUID_TAIL);
        }
    }
    Chunk::new(*b"fmt ", p)
}

/// A `data` chunk holding `samples` little-endian.
#[must_use]
pub fn data(samples: &Samples) -> Chunk {
    Chunk::new(*b"data", bytes_le(samples))
}

/// A `fact` chunk (`dwSampleLength`, frames).
#[must_use]
pub fn fact(frames: u32) -> Chunk {
    Chunk::new(*b"fact", frames.to_le_bytes().to_vec())
}

/// A `LIST` chunk of form `INFO` with NUL-terminated text sub-chunks (each padded to even).
#[must_use]
pub fn list_info(items: &[(&[u8; 4], &str)]) -> Chunk {
    let mut p = b"INFO".to_vec();
    for (id, text) in items {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        p.extend_from_slice(*id);
        let len = u32::try_from(value.len()).expect("INFO texts are short");
        p.extend_from_slice(&len.to_le_bytes());
        p.extend_from_slice(&value);
        if value.len() % 2 == 1 {
            p.push(0);
        }
    }
    Chunk::new(*b"LIST", p)
}

fn fixed(text: &str, width: usize) -> Vec<u8> {
    let mut v = text.as_bytes().to_vec();
    v.resize(width, 0);
    v
}

/// Byte offset of `TimeReference` (u64 LE) in a `bext` payload.
pub const BEXT_TIME_REFERENCE: usize = 338;
/// Byte offset of `Version` (u16 LE).
pub const BEXT_VERSION: usize = 346;
/// Byte offset of the five v2 loudness fields (i16 LE each, 0.01 units).
pub const BEXT_LOUDNESS: usize = 412;

/// A BWF `bext` chunk (EBU Tech 3285): 602 bytes plus coding history. Version 0 leaves the UMID
/// and loudness bytes reserved (zero); version 1 fills the UMID; version 2 also the loudness
/// fields (`LoudnessValue` -16.00 LUFS, `LoudnessRange` 5.00 LU, `MaxTruePeakLevel` -1.00 dBTP,
/// `MaxMomentaryLoudness` -12.00, `MaxShortTermLoudness` -14.00).
#[must_use]
pub fn bext(version: u16, sample_rate: u32, coding_history: &str) -> Chunk {
    let mut p = fixed("SoundCheck matrix fixture", 256);
    p.extend(fixed("SoundCheck", 32));
    p.extend(fixed("SCMATRIX0001", 32));
    p.extend(fixed("2026-10-03", 10));
    p.extend(fixed("12:00:00", 8));
    p.extend_from_slice(&(u64::from(sample_rate) * 3600).to_le_bytes()); // TimeReference: 1 h
    p.extend_from_slice(&version.to_le_bytes());
    if version >= 1 {
        // A basic SMPTE 330M UMID: universal label, length 0x13, instance, material number.
        let mut umid = vec![
            0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0D, 0x20, 0x13,
        ];
        umid.extend((0_u8..19).map(|i| i.wrapping_mul(37)));
        umid.resize(64, 0);
        p.extend(umid);
    } else {
        p.extend(std::iter::repeat_n(0_u8, 64));
    }
    if version >= 2 {
        for v in [-1600_i16, 500, -100, -1200, -1400] {
            p.extend_from_slice(&v.to_le_bytes());
        }
    } else {
        p.extend(std::iter::repeat_n(0_u8, 10));
    }
    p.extend(std::iter::repeat_n(0_u8, 180)); // reserved
    p.extend_from_slice(coding_history.as_bytes());
    Chunk::new(*b"bext", p)
}

/// An `iXML` chunk.
#[must_use]
pub fn ixml() -> Chunk {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<BWFXML><IXML_VERSION>1.5\
               </IXML_VERSION><PROJECT>matrix</PROJECT><SCENE>1</SCENE><TAKE>2</TAKE>\
               <NOTE>carried verbatim</NOTE></BWFXML>\n";
    Chunk::new(*b"iXML", xml.as_bytes().to_vec())
}

/// A `cue ` chunk with one point per position (ids from 1).
#[must_use]
pub fn cue(positions: &[u32]) -> Chunk {
    let count = u32::try_from(positions.len()).expect("few cue points");
    let mut p = count.to_le_bytes().to_vec();
    for (id, pos) in (1_u32..).zip(positions) {
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(&pos.to_le_bytes());
        p.extend_from_slice(b"data");
        p.extend_from_slice(&0_u32.to_le_bytes()); // chunk start
        p.extend_from_slice(&0_u32.to_le_bytes()); // block start
        p.extend_from_slice(&pos.to_le_bytes()); // sample offset
    }
    Chunk::new(*b"cue ", p)
}

/// An `smpl` chunk with one forward loop.
#[must_use]
pub fn smpl(sample_rate: u32, loop_start: u32, loop_end: u32) -> Chunk {
    let fields = [
        0,                           // manufacturer
        0,                           // product
        1_000_000_000 / sample_rate, // sample period, ns
        60,                          // MIDI unity note
        0,                           // pitch fraction
        0,                           // SMPTE format
        0,                           // SMPTE offset
        1,                           // loops
        0,                           // sampler data
        1,                           // loop: cue point id
        0,                           // loop: type (forward)
        loop_start,
        loop_end,
        0, // fraction
        0, // play count (infinite)
    ];
    Chunk::new(
        *b"smpl",
        fields.iter().flat_map(|v: &u32| v.to_le_bytes()).collect(),
    )
}

/// Options for [`build`].
#[derive(Debug, Clone, Default)]
pub struct RiffLayout {
    /// Write `RF64` with a `ds64` chunk holding the real sizes and this frame count.
    pub rf64_frames: Option<u64>,
    /// Leave out the pad byte after an odd-length `data` chunk.
    pub omit_odd_data_pad: bool,
    /// Bytes appended after the RIFF end.
    pub trailing: Vec<u8>,
}

fn size32(len: usize) -> [u8; 4] {
    u32::try_from(len)
        .expect("fixtures stay far below 4 GiB")
        .to_le_bytes()
}

/// Writes a WAV (or RF64) file from chunks in order.
#[must_use]
pub fn build(chunks: &[Chunk], layout: &RiffLayout) -> Built {
    let rf64 = layout.rf64_frames.is_some();
    let mut body = Vec::new();
    let mut listing = Vec::new();
    let mut data_len = 0_usize;
    for chunk in chunks {
        let is_data = &chunk.id == b"data";
        let odd = chunk.payload.len() % 2 == 1;
        let pad = odd && !(is_data && layout.omit_odd_data_pad);
        body.extend_from_slice(&chunk.id);
        if rf64 && is_data {
            body.extend_from_slice(&u32::MAX.to_le_bytes());
        } else {
            body.extend_from_slice(&size32(chunk.payload.len()));
        }
        body.extend_from_slice(&chunk.payload);
        if pad {
            body.push(chunk.pad);
        }
        if is_data {
            data_len = chunk.payload.len();
        }
        listing.push(chunk.listed(pad));
        listing.extend(chunk.nested.iter().cloned());
    }
    let mut bytes = Vec::with_capacity(body.len() + 64);
    if let Some(frames) = layout.rf64_frames {
        let mut ds64 = Vec::new();
        let riff_size = 4 + 8 + 28 + body.len() as u64;
        ds64.extend_from_slice(&riff_size.to_le_bytes());
        ds64.extend_from_slice(&(data_len as u64).to_le_bytes());
        ds64.extend_from_slice(&frames.to_le_bytes());
        ds64.extend_from_slice(&0_u32.to_le_bytes()); // table length
        let ds64 = Chunk::new(*b"ds64", ds64);
        bytes.extend_from_slice(b"RF64");
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"ds64");
        bytes.extend_from_slice(&size32(ds64.payload.len()));
        bytes.extend_from_slice(&ds64.payload);
        listing.insert(0, ds64.listed(false));
    } else {
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&size32(4 + body.len()));
        bytes.extend_from_slice(b"WAVE");
    }
    bytes.extend_from_slice(&body);
    if !layout.trailing.is_empty() {
        bytes.extend_from_slice(&layout.trailing);
        listing.push(Listed {
            kind: Kind::Trailing,
            id: "trailing".into(),
            sha256: sha256_hex(&layout.trailing),
            pad: None,
        });
    }
    Built { bytes, listing }
}

/// An `ID3v1` tag (128 bytes: `TAG`, title, artist, album, year, comment, genre) as some taggers
/// append it after the RIFF end.
#[must_use]
pub fn id3v1(title: &str, artist: &str) -> Vec<u8> {
    let mut v = b"TAG".to_vec();
    v.extend(fixed(title, 30));
    v.extend(fixed(artist, 30));
    v.extend(fixed("Matrix", 30));
    v.extend(fixed("2026", 4));
    v.extend(fixed("carried after RIFF", 30));
    v.push(52); // genre: Electronic
    v
}
