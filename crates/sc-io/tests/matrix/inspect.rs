//! Header and audio inspection on top of `parse.rs`: WAV `fmt ` (Microsoft RIFF, WAVEFORMATEX
//! and WAVEFORMATEXTENSIBLE), AIFF/AIFF-C `COMM` with its 80-bit IEEE 754 extended sample rate
//! and `SSND`, the PCM stored in either, FLAC STREAMINFO and SEEKTABLE (RFC 9639 sections 8.2
//! and 8.5), a frame walk that finds every FLAC frame by its header CRC-8 and footer CRC-16
//! (RFC 9639 section 9), and the STREAMINFO MD5 of a PCM signal.
//!
//! The CRCs here are table-driven, unlike the bitwise ones in the fixture writer, so a mistake
//! in one does not cancel out in the other.

use md5::{Digest, Md5};

use super::parse::{Container, Kind, Parsed};
use super::pcm::Samples;

/// Fields of a WAV `fmt ` chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavFmt {
    /// Chunk payload length, bytes (16 for plain PCM, 18 or 40 otherwise).
    pub len: usize,
    /// `wFormatTag`: 0x0001 PCM, 0x0003 IEEE float, 0xFFFE extensible.
    pub format_tag: u16,
    /// Channel count.
    pub channels: u16,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Bytes per second.
    pub byte_rate: u32,
    /// Bytes per frame.
    pub block_align: u16,
    /// Bits per sample (container width).
    pub bits: u16,
    /// First two bytes of the extensible sub-format GUID (the format tag it stands for).
    pub sub_format: Option<u16>,
}

fn u16_le(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| format!("short field at {at}"))
}

fn u32_le(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("short field at {at}"))
}

fn u16_be(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or_else(|| format!("short field at {at}"))
}

fn u32_be(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("short field at {at}"))
}

/// Reads a `fmt ` payload.
///
/// # Errors
/// When the payload is shorter than its fields.
pub fn wav_fmt(p: &[u8]) -> Result<WavFmt, String> {
    let format_tag = u16_le(p, 0)?;
    Ok(WavFmt {
        len: p.len(),
        format_tag,
        channels: u16_le(p, 2)?,
        sample_rate: u32_le(p, 4)?,
        byte_rate: u32_le(p, 8)?,
        block_align: u16_le(p, 12)?,
        bits: u16_le(p, 14)?,
        sub_format: if format_tag == 0xFFFE {
            Some(u16_le(p, 24)?)
        } else {
            None
        },
    })
}

/// Fields of an AIFF/AIFF-C `COMM` chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct AiffComm {
    /// Chunk payload length, bytes (18 for AIFF).
    pub len: usize,
    /// Channel count.
    pub channels: u16,
    /// Sample frames.
    pub frames: u32,
    /// Bits per sample.
    pub bits: u16,
    /// Sample rate, Hz, decoded from the 80-bit extended field.
    pub sample_rate: f64,
    /// AIFF-C compression type (`NONE`, `sowt`, ...), `None` for AIFF.
    pub compression: Option<[u8; 4]>,
}

/// Decodes an 80-bit IEEE 754 extended value (sign, 15-bit exponent biased by 16383, 64-bit
/// mantissa with an explicit integer bit).
#[must_use]
pub fn extended_to_f64(b: &[u8; 10]) -> f64 {
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(u16::from_be_bytes([b[0] & 0x7F, b[1]])) - 16383;
    let mut mantissa = [0_u8; 8];
    mantissa.copy_from_slice(&b[2..]);
    let mantissa = u64::from_be_bytes(mantissa);
    // Value = mantissa * 2^(exponent - 63); sample rates are integers well inside f64's range.
    #[allow(clippy::cast_precision_loss)]
    let m = mantissa as f64;
    sign * m * 2_f64.powi(exponent - 63)
}

/// Reads a `COMM` payload (`aifc` adds the compression type).
///
/// # Errors
/// When the payload is shorter than its fields.
pub fn aiff_comm(p: &[u8], aifc: bool) -> Result<AiffComm, String> {
    let rate: [u8; 10] = p
        .get(8..18)
        .and_then(|s| s.try_into().ok())
        .ok_or("short COMM")?;
    let compression = if aifc {
        Some(
            p.get(18..22)
                .and_then(|s| s.try_into().ok())
                .ok_or("AIFC COMM without compression type")?,
        )
    } else {
        None
    };
    Ok(AiffComm {
        len: p.len(),
        channels: u16_be(p, 0)?,
        frames: u32_be(p, 2)?,
        bits: u16_be(p, 6)?,
        sample_rate: extended_to_f64(&rate),
        compression,
    })
}

/// PCM as stored in a WAV or AIFF file.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPcm {
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Sample frames.
    pub frames: usize,
    /// Interleaved samples.
    pub samples: Samples,
}

fn int_pcm(bytes: &[u8], bits: u16, little: bool) -> Result<Vec<i32>, String> {
    let width = usize::from(bits / 8);
    if !(bits == 16 || bits == 24) || !bytes.len().is_multiple_of(width) {
        return Err(format!("{bits}-bit data of {} bytes", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(width)
        .map(|c| {
            let mut v = 0_i32;
            for i in 0..width {
                let byte = if little { c[width - 1 - i] } else { c[i] };
                v = (v << 8) | i32::from(byte);
            }
            // Sign-extend from `bits`.
            let shift = 32 - u32::from(bits);
            (v << shift) >> shift
        })
        .collect())
}

/// The audio of a WAV/RF64/AIFF/AIFF-C file read straight from its `data`/`SSND` chunk.
///
/// # Errors
/// When the header chunks are missing or describe something other than 16/24-bit integer or
/// 32-bit float PCM.
pub fn iff_pcm(parsed: &Parsed) -> Result<StoredPcm, String> {
    let chunk = |id: &str| {
        parsed
            .find(Kind::Chunk, id)
            .map(|b| b.bytes.as_slice())
            .ok_or_else(|| format!("no {id:?} chunk"))
    };
    let (rate, channels, bits, float, little, audio) = match parsed.container {
        Container::Wave | Container::Rf64 => {
            let fmt = wav_fmt(chunk("fmt ")?)?;
            let float = fmt.format_tag == 3 || fmt.sub_format == Some(3);
            let audio = chunk("data")?;
            (fmt.sample_rate, fmt.channels, fmt.bits, float, true, audio)
        }
        Container::Aiff | Container::Aifc => {
            let comm = aiff_comm(chunk("COMM")?, parsed.container == Container::Aifc)?;
            let ssnd = chunk("SSND")?;
            let offset = usize::try_from(u32_be(ssnd, 0)?).map_err(|e| e.to_string())?;
            let audio = ssnd.get(8 + offset..).ok_or("SSND offset past its end")?;
            let little = comm.compression == Some(*b"sowt");
            // Rates are small integers; the f64 holds them exactly.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let rate = comm.sample_rate as u32;
            (rate, comm.channels, comm.bits, false, little, audio)
        }
        Container::Flac => return Err("FLAC has no IFF audio chunk".into()),
    };
    let samples = if float {
        Samples::Float(
            audio
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
        )
    } else {
        Samples::Int {
            bits: u8::try_from(bits).map_err(|e| e.to_string())?,
            data: int_pcm(audio, bits, little)?,
        }
    };
    Ok(StoredPcm {
        sample_rate: rate,
        channels,
        frames: samples.len() / usize::from(channels.max(1)),
        samples,
    })
}

/// Fields of a FLAC STREAMINFO block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// Minimum block size, samples (the last block excluded).
    pub min_block: u16,
    /// Maximum block size, samples.
    pub max_block: u16,
    /// Minimum frame size, bytes (0 = unknown).
    pub min_frame: u32,
    /// Maximum frame size, bytes (0 = unknown).
    pub max_frame: u32,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u8,
    /// Bits per sample.
    pub bits: u8,
    /// Total samples per channel.
    pub total_samples: u64,
    /// MD5 of the unencoded audio.
    pub md5: [u8; 16],
}

/// Reads a STREAMINFO payload (34 bytes).
///
/// # Errors
/// When the payload is not 34 bytes long.
pub fn streaminfo(p: &[u8]) -> Result<StreamInfo, String> {
    if p.len() != 34 {
        return Err(format!("STREAMINFO of {} bytes", p.len()));
    }
    let be24 = |i: usize| u32::from(p[i]) << 16 | u32::from(p[i + 1]) << 8 | u32::from(p[i + 2]);
    let mut packed = [0_u8; 8];
    packed.copy_from_slice(&p[10..18]);
    let packed = u64::from_be_bytes(packed);
    let mut md5 = [0_u8; 16];
    md5.copy_from_slice(&p[18..34]);
    // Each field is masked to its width before narrowing.
    #[allow(clippy::cast_possible_truncation)]
    Ok(StreamInfo {
        min_block: u16_be(p, 0)?,
        max_block: u16_be(p, 2)?,
        min_frame: be24(4),
        max_frame: be24(7),
        sample_rate: (packed >> 44) as u32,
        channels: ((packed >> 41) & 0x7) as u8 + 1,
        bits: ((packed >> 36) & 0x1F) as u8 + 1,
        total_samples: packed & 0xF_FFFF_FFFF,
        md5,
    })
}

/// One SEEKTABLE point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekPoint {
    /// First sample of the target frame; `u64::MAX` marks a placeholder.
    pub sample: u64,
    /// Byte offset of the target frame from the first frame header.
    pub offset: u64,
    /// Samples in the target frame.
    pub frame_samples: u16,
}

/// Reads a SEEKTABLE payload (18 bytes per point).
///
/// # Errors
/// When the payload length is not a multiple of 18.
pub fn seektable(p: &[u8]) -> Result<Vec<SeekPoint>, String> {
    if !p.len().is_multiple_of(18) {
        return Err(format!("SEEKTABLE of {} bytes", p.len()));
    }
    Ok(p.as_chunks::<18>()
        .0
        .iter()
        .map(|c| {
            let mut a = [0_u8; 8];
            a.copy_from_slice(&c[..8]);
            let mut o = [0_u8; 8];
            o.copy_from_slice(&c[8..16]);
            SeekPoint {
                sample: u64::from_be_bytes(a),
                offset: u64::from_be_bytes(o),
                frame_samples: u16::from_be_bytes([c[16], c[17]]),
            }
        })
        .collect())
}

const fn crc8_table() -> [u8; 256] {
    let mut t = [0_u8; 256];
    let mut i = 0;
    while i < 256 {
        // `i < 256` keeps the narrowing exact.
        #[allow(clippy::cast_possible_truncation)]
        let mut c = i as u8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x80 != 0 {
                (c << 1) ^ 0x07
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const fn crc16_table() -> [u16; 256] {
    let mut t = [0_u16; 256];
    let mut i = 0;
    while i < 256 {
        // `i < 256` keeps the narrowing exact.
        #[allow(clippy::cast_possible_truncation)]
        let mut c = (i as u16) << 8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000 != 0 {
                (c << 1) ^ 0x8005
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC8: [u8; 256] = crc8_table();
const CRC16: [u16; 256] = crc16_table();

fn crc16_step(crc: u16, byte: u8) -> u16 {
    (crc << 8) ^ CRC16[usize::from(crc.to_be_bytes()[0] ^ byte)]
}

/// One FLAC frame found by [`flac_frames`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInfo {
    /// Offset from the first frame header, bytes.
    pub offset: usize,
    /// Frame length including header and CRC-16, bytes.
    pub len: usize,
    /// First sample (per channel) of the frame.
    pub first_sample: u64,
    /// Samples per channel in the frame.
    pub block_size: u32,
}

/// Decodes the "UTF-8" coded frame or sample number at the start of `b` (RFC 9639 section
/// 9.1.5: up to 7 bytes, 36 bits); returns the number and the bytes used.
#[must_use]
pub fn decode_coded_number(b: &[u8]) -> Option<(u64, usize)> {
    let first = *b.first()?;
    let extra = first.leading_ones() as usize;
    if extra == 1 || extra > 7 {
        return None;
    }
    let n_len = if extra == 0 { 1 } else { extra };
    let mut number = u64::from(first & (0x7F >> extra));
    for i in 1..n_len {
        let c = *b.get(i)?;
        if c & 0xC0 != 0x80 {
            return None;
        }
        number = (number << 6) | u64::from(c & 0x3F);
    }
    Some((number, n_len))
}

/// Parses a frame header at `b[0..]`; returns (header length, coded number, block size,
/// variable blocking) when the sync code and CRC-8 are valid.
fn frame_header(b: &[u8]) -> Option<(usize, u64, u32, bool)> {
    if b.len() < 6 || b[0] != 0xFF || b[1] & 0xFE != 0xF8 {
        return None;
    }
    let variable = b[1] & 1 == 1;
    let (bs_code, sr_code) = (b[2] >> 4, b[2] & 0x0F);
    let (number, n_len) = decode_coded_number(&b[4..])?;
    let mut pos = 4 + n_len;
    let block_size = match bs_code {
        1 => 192,
        2..=5 => 576 << (bs_code - 2),
        6 => {
            pos += 1;
            u32::from(*b.get(pos - 1)?) + 1
        }
        7 => {
            pos += 2;
            u32::from(u16::from_be_bytes([*b.get(pos - 2)?, *b.get(pos - 1)?])) + 1
        }
        8..=15 => 256 << (bs_code - 8),
        _ => return None,
    };
    pos += match sr_code {
        12 => 1,
        13 | 14 => 2,
        15 => return None,
        _ => 0,
    };
    let crc = b
        .get(..pos)?
        .iter()
        .fold(0_u8, |c, x| CRC8[usize::from(c ^ x)]);
    (crc == *b.get(pos)?).then_some((pos + 1, number, block_size, variable))
}

/// Walks the FLAC frames in `region` (everything after the metadata blocks): a frame ends where
/// the next valid header starts and the CRC-16 over the frame matches its last two bytes.
///
/// # Errors
/// When the region does not start with a frame header or a frame's CRC-16 never matches.
pub fn flac_frames(region: &[u8], fixed_block: u32) -> Result<Vec<FrameInfo>, String> {
    let mut frames = Vec::new();
    let mut pos = 0;
    while pos < region.len() {
        let (head_len, number, block_size, variable) = frame_header(&region[pos..])
            .ok_or_else(|| format!("no valid frame header at {pos}"))?;
        let mut crc = 0_u16;
        let mut end = None;
        for j in pos..region.len() {
            // `crc` covers region[pos..j]; a frame ending at j + 2 needs its footer to match.
            if j > pos + head_len && j + 2 <= region.len() {
                let footer = u16::from_be_bytes([region[j], region[j + 1]]);
                let at_end = j + 2 == region.len();
                if footer == crc && (at_end || frame_header(&region[j + 2..]).is_some()) {
                    end = Some(j + 2);
                    break;
                }
            }
            crc = crc16_step(crc, region[j]);
        }
        let end = end.ok_or_else(|| format!("no CRC-16 match for the frame at {pos}"))?;
        let first_sample = if variable {
            number
        } else {
            number * u64::from(fixed_block)
        };
        frames.push(FrameInfo {
            offset: pos,
            len: end - pos,
            first_sample,
            block_size,
        });
        pos = end;
    }
    Ok(frames)
}

/// MD5 of interleaved samples as signed little-endian integers of `ceil(bits / 8)` bytes, as
/// STREAMINFO stores it.
#[must_use]
pub fn pcm_md5(samples: &[i32], bits: u8) -> [u8; 16] {
    let width = usize::from(bits.div_ceil(8));
    let mut md5 = Md5::new();
    for s in samples {
        md5.update(&s.to_le_bytes()[..width]);
    }
    md5.finalize().into()
}
