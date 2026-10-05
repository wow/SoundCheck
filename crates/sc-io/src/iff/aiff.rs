//! AIFF/AIFF-C `COMM` and `SSND` decoding (Apple AIFF 1.3: `numChannels`, `numSampleFrames`,
//! `sampleSize`, 80-bit extended `sampleRate`; AIFF-C draft 1991: `compressionType` and its
//! Pascal-string name; `SSND`: `offset`, `blockSize`, then sound data). Samples are big-endian
//! two's complement, left-justified in `ceil(sampleSize / 8)` bytes; AIFF-C `sowt` stores them
//! little-endian, `fl32`/`fl64` as big-endian IEEE floats.

use std::io::{Read, Seek};

use sc_core::Result;

use super::extended::{ExtendedRateError, sample_rate_from_extended};
use super::format::{AudioFormat, SampleEncoding, check_rate, whole_frames};
use super::walk::{Chunk, ChunkTable};
use super::{Container, Source, capped, corrupt, fourcc, unsupported};

/// `COMM` bytes of plain AIFF.
const COMM_BYTES: usize = 18;
/// `COMM` bytes of AIFF-C up to and including the compression type.
const COMM_AIFC_BYTES: usize = 22;
/// `SSND` fields before the sound data: offset and block size.
const SSND_FIELDS_BYTES: u64 = 8;

/// Decodes `COMM` and locates the sound data in `SSND`.
pub(super) fn read<R: Read + Seek>(
    src: &mut Source<'_, R>,
    table: &ChunkTable,
) -> Result<AudioFormat> {
    let path = src.path();
    let format_chunk = table
        .position(b"COMM")
        .ok_or_else(|| corrupt(path, 12, "no COMM chunk"))?;
    let comm = &table.chunks[format_chunk];
    let aifc = table.container == Container::Aifc;
    let need = if aifc { COMM_AIFC_BYTES } else { COMM_BYTES };
    let mut p = [0_u8; COMM_AIFC_BYTES];
    let n = capped(comm.payload_len(), need);
    if n < need {
        let what = if aifc {
            "AIFF-C COMM chunk shorter than 22 bytes"
        } else {
            "COMM chunk shorter than 18 bytes"
        };
        return Err(corrupt(path, comm.header_offset, what));
    }
    src.read_at(comm.payload.start, &mut p[..n])?;
    let channels = i16::from_be_bytes([p[0], p[1]]);
    let frames_declared = u64::from(u32::from_be_bytes([p[2], p[3], p[4], p[5]]));
    let sample_size = i16::from_be_bytes([p[6], p[7]]);
    let mut rate = [0_u8; 10];
    rate.copy_from_slice(&p[8..18]);
    let sample_rate = match sample_rate_from_extended(rate) {
        Ok(hz) => check_rate(path, comm.header_offset, hz)?,
        Err(e @ ExtendedRateError::NotInteger(_)) => return Err(unsupported(path, e.to_string())),
        Err(e) => return Err(corrupt(path, comm.header_offset, &e.to_string())),
    };
    let channels = u16::try_from(channels)
        .ok()
        .filter(|c| *c > 0)
        .ok_or_else(|| corrupt(path, comm.header_offset, "COMM with no channels"))?;
    let compression = aifc.then(|| [p[18], p[19], p[20], p[21]]);
    let encoding = match compression.as_ref() {
        None | Some(b"NONE" | b"twos") => SampleEncoding::IntBe,
        Some(b"sowt") => SampleEncoding::IntLe,
        Some(b"fl32" | b"FL32") => SampleEncoding::FloatBe32,
        Some(b"fl64" | b"FL64") => SampleEncoding::FloatBe64,
        Some(other) => {
            let name = compression_name(src, comm)?;
            return Err(unsupported(
                path,
                format!("AIFF-C compression '{}' ({name})", fourcc(*other)),
            ));
        }
    };
    let (container_bits, valid_bits) = match encoding {
        SampleEncoding::FloatBe32 => (32, 32),
        SampleEncoding::FloatBe64 => (64, 64),
        _ => match u16::try_from(sample_size) {
            Ok(bits @ 1..=32) => (bits.div_ceil(8) * 8, bits),
            _ => return Err(unsupported(path, format!("{sample_size}-bit AIFF"))),
        },
    };
    let block_align =
        u16::try_from(u32::from(channels) * u32::from(container_bits / 8)).map_err(|_| {
            unsupported(
                path,
                format!("{channels} channels of {container_bits} bits"),
            )
        })?;
    let audio_chunk = table.position(b"SSND");
    let (frames, data, mismatch) = match audio_chunk {
        Some(i) => sound_data(src, &table.chunks[i], block_align, frames_declared)?,
        // AIFF 1.3: the SSND chunk may be absent when numSampleFrames is zero.
        None if frames_declared == 0 => (0, comm.payload.end..comm.payload.end, false),
        None => return Err(corrupt(path, 12, "no SSND chunk")),
    };
    Ok(AudioFormat {
        sample_rate,
        channels,
        bits_per_sample: container_bits,
        valid_bits,
        encoding,
        block_align,
        frames,
        frames_declared: Some(frames_declared),
        frames_mismatch: mismatch,
        data,
        channel_mask: None,
        format_tag: None,
        aifc_compression: compression,
        format_chunk,
        audio_chunk,
    })
}

/// Frames, audio byte range and whether `COMM` disagrees with `SSND`.
fn sound_data<R: Read + Seek>(
    src: &mut Source<'_, R>,
    ssnd: &Chunk,
    block_align: u16,
    frames_declared: u64,
) -> Result<(u64, std::ops::Range<u64>, bool)> {
    let path = src.path();
    if ssnd.payload_len() < SSND_FIELDS_BYTES {
        return Err(corrupt(
            path,
            ssnd.header_offset,
            "SSND chunk shorter than 8 bytes",
        ));
    }
    let mut fields = [0_u8; 8];
    src.read_at(ssnd.payload.start, &mut fields)?;
    let offset = u64::from(u32::from_be_bytes([
        fields[0], fields[1], fields[2], fields[3],
    ]));
    let start = ssnd.payload.start + SSND_FIELDS_BYTES + offset;
    if start > ssnd.payload.end {
        return Err(corrupt(
            path,
            ssnd.header_offset,
            &format!("SSND offset {offset} runs past the chunk"),
        ));
    }
    let (stored, _) = whole_frames(start..ssnd.payload.end, block_align);
    let frames = stored.min(frames_declared);
    let data = start..start + frames * u64::from(block_align);
    Ok((frames, data, stored != frames_declared))
}

/// The AIFF-C compression name (Pascal string after the type) for messages; empty when absent.
fn compression_name<R: Read + Seek>(src: &mut Source<'_, R>, comm: &Chunk) -> Result<String> {
    let at = comm.payload.start + COMM_AIFC_BYTES as u64;
    let available = comm.payload.end.saturating_sub(at);
    let mut name = [0_u8; 256];
    let n = capped(available, name.len());
    if n == 0 {
        return Ok(String::new());
    }
    src.read_at(at, &mut name[..n])?;
    let len = usize::from(name[0]).min(n - 1);
    Ok(String::from_utf8_lossy(&name[1..=len]).into_owned())
}
