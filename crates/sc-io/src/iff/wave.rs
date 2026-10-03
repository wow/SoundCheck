//! WAV/RF64 `fmt ` decoding: `WAVEFORMATEX` (`wFormatTag` 0x0001 PCM, 0x0003 IEEE float) and
//! `WAVEFORMATEXTENSIBLE` (0xFFFE: `wValidBitsPerSample`, `dwChannelMask`, sub-format GUID
//! `KSDATAFORMAT_SUBTYPE_PCM` or `_IEEE_FLOAT`), per Microsoft's RIFF/WAVE and
//! `WAVEFORMATEXTENSIBLE` documentation. RF64 shares the layout (EBU Tech 3306).

use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::path::Path;

use sc_core::Result;

use super::format::{AudioFormat, SampleEncoding, check_rate, whole_frames};
use super::walk::{Chunk, ChunkTable};
use super::{Source, capped, corrupt, unsupported};

/// `WAVE_FORMAT_PCM`.
const TAG_PCM: u16 = 0x0001;
/// `WAVE_FORMAT_IEEE_FLOAT`.
const TAG_FLOAT: u16 = 0x0003;
/// `WAVE_FORMAT_EXTENSIBLE`.
const TAG_EXTENSIBLE: u16 = 0xFFFE;
/// Bytes 4..16 shared by every `KSDATAFORMAT_SUBTYPE_*` GUID derived from a format tag
/// (xxxxxxxx-0000-0010-8000-00AA00389B71, little-endian fields).
const KS_GUID_TAIL: [u8; 12] = [
    0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];
/// Bytes of an extensible `fmt ` payload that are read.
const EXTENSIBLE_BYTES: usize = 40;

fn u16_le(p: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([p[at], p[at + 1]])
}

fn u32_le(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

/// Decodes `fmt ` and locates `data`.
pub(super) fn read<R: Read + Seek>(
    src: &mut Source<'_, R>,
    table: &ChunkTable,
) -> Result<AudioFormat> {
    let path = src.path();
    let fmt = table
        .find(b"fmt ")
        .ok_or_else(|| corrupt(path, 12, "no fmt chunk"))?;
    let data = table
        .find(b"data")
        .ok_or_else(|| corrupt(path, 12, "no data chunk"))?;
    let mut p = [0_u8; EXTENSIBLE_BYTES];
    let n = capped(fmt.payload_len(), EXTENSIBLE_BYTES);
    if n < 16 {
        return Err(corrupt(
            path,
            fmt.header_offset,
            "fmt chunk shorter than 16 bytes",
        ));
    }
    src.read_at(fmt.payload.start, &mut p[..n])?;
    let tag = u16_le(&p, 0);
    let channels = u16_le(&p, 2);
    let sample_rate = check_rate(path, fmt.header_offset, u32_le(&p, 4))?;
    let block_align = u16_le(&p, 12);
    let bits = u16_le(&p, 14);
    if channels == 0 {
        return Err(corrupt(
            path,
            fmt.header_offset,
            "fmt chunk with 0 channels",
        ));
    }
    let (code, valid, channel_mask) = if tag == TAG_EXTENSIBLE {
        extensible(path, fmt, &p[..n])?
    } else {
        (tag, bits, None)
    };
    let (encoding, container_bits, valid_bits) = match code {
        TAG_PCM => int_layout(path, fmt, channels, block_align, bits, valid, tag)?,
        TAG_FLOAT => float_layout(path, fmt, channels, block_align, bits)?,
        _ if tag == TAG_EXTENSIBLE => {
            return Err(unsupported(
                path,
                format!("WAV extensible sub-format 0x{code:04X}"),
            ));
        }
        _ => return Err(unsupported(path, format!("WAV format tag 0x{tag:04X}"))),
    };
    let (frames, range) = whole_frames(data.payload.clone(), block_align);
    Ok(AudioFormat {
        sample_rate,
        channels,
        bits_per_sample: container_bits,
        valid_bits,
        encoding,
        block_align,
        frames,
        frames_declared: None,
        frames_mismatch: false,
        data: range,
        channel_mask,
        format_tag: Some(tag),
        aifc_compression: None,
    })
}

/// Sub-format code, valid bits and channel mask of an extensible `fmt `.
fn extensible(path: &Path, fmt: &Chunk, p: &[u8]) -> Result<(u16, u16, Option<u32>)> {
    if p.len() < EXTENSIBLE_BYTES {
        return Err(corrupt(
            path,
            fmt.header_offset,
            "extensible fmt chunk shorter than 40 bytes",
        ));
    }
    let guid = &p[24..40];
    let code = u32_le(guid, 0);
    if guid[4..] != KS_GUID_TAIL {
        let hex = guid.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02X}");
            s
        });
        return Err(unsupported(
            path,
            format!("WAV extensible sub-format GUID {hex}"),
        ));
    }
    let code = u16::try_from(code)
        .map_err(|_| unsupported(path, format!("WAV extensible sub-format 0x{code:08X}")))?;
    Ok((code, u16_le(p, 18), Some(u32_le(p, 20))))
}

/// Encoding, container bits and valid bits of integer PCM.
///
/// `bits` is `wBitsPerSample`; `valid` is `wValidBitsPerSample` for extensible (0 means "same
/// as the container") and `bits` otherwise. The container is `block_align / channels` bytes
/// when that holds the bits in at most 4 bytes (a 20-bit plain-PCM file in 3 or 4 bytes);
/// anything else is corrupt.
fn int_layout(
    path: &Path,
    fmt: &Chunk,
    channels: u16,
    block_align: u16,
    bits: u16,
    valid: u16,
    tag: u16,
) -> Result<(SampleEncoding, u16, u16)> {
    let extensible = tag == TAG_EXTENSIBLE;
    if bits == 0 || bits > 32 || (extensible && !bits.is_multiple_of(8)) {
        return Err(unsupported(path, format!("{bits}-bit integer WAV")));
    }
    let needed = bits.div_ceil(8);
    let per_channel = block_align / channels;
    let fits = block_align.is_multiple_of(channels) && (needed..=4).contains(&per_channel);
    if !fits || (extensible && per_channel != needed) {
        return Err(corrupt(
            path,
            fmt.header_offset,
            &format!("block align {block_align} does not hold {channels} channels of {bits} bits"),
        ));
    }
    let container = per_channel * 8;
    let valid = if extensible && valid != 0 {
        valid
    } else {
        bits
    };
    if valid > container {
        return Err(corrupt(
            path,
            fmt.header_offset,
            &format!("{valid} valid bits in a {container}-bit container"),
        ));
    }
    let encoding = if per_channel == 1 {
        SampleEncoding::UnsignedInt8
    } else {
        SampleEncoding::IntLe
    };
    Ok((encoding, container, valid))
}

/// Encoding, container bits and valid bits of IEEE float samples.
fn float_layout(
    path: &Path,
    fmt: &Chunk,
    channels: u16,
    block_align: u16,
    bits: u16,
) -> Result<(SampleEncoding, u16, u16)> {
    let encoding = match bits {
        32 => SampleEncoding::FloatLe32,
        64 => SampleEncoding::FloatLe64,
        _ => return Err(unsupported(path, format!("{bits}-bit float WAV"))),
    };
    if u32::from(block_align) != u32::from(channels) * u32::from(bits / 8) {
        return Err(corrupt(
            path,
            fmt.header_offset,
            &format!("block align {block_align} does not hold {channels} channels of {bits} bits"),
        ));
    }
    Ok((encoding, bits, bits))
}
