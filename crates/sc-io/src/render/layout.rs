//! The output's chunk list, decided before a byte is written: which source chunks are carried
//! by byte range, which are patched, replaced or dropped, every size, and the refusals that
//! follow from the chunks (duplicated format or audio chunks, malformed position chunks, a
//! sampler loop inside the trim, more patched bytes than [`MAX_PATCHED_TOTAL_BYTES`], an
//! output past the 4 GiB limit of a 32-bit RIFF size or the 2 GiB limit of AIFF's signed
//! `ckSize`).

use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use sc_core::{Error, Result};

use super::patch::{self, BextUpdate, PatchError};
use super::{BlockFate, BlockRecord};
use crate::iff::{
    AudioFormat, Chunk, ChunkTable, OutContainer, SSND_FIELDS_BYTES, aiff_comm, wave_fmt_pcm,
};

/// Largest chunk read into memory to be patched (a `bext` with a long coding history fits
/// many times over); larger ones are refused rather than buffered.
pub const MAX_PATCHED_CHUNK_BYTES: u64 = 4 << 20;

/// Most patched bytes one plan holds in memory, all chunks together.
pub const MAX_PATCHED_TOTAL_BYTES: u64 = 16 << 20;

/// Largest AIFF container size: AIFF 1.3 declares `ckSize` as a signed 32-bit `long`.
pub const MAX_AIFF_FORM_SIZE: u64 = 0x7FFF_FFFF;

/// What the output holds and how it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    /// Output container.
    pub container: OutContainer,
    /// Channels (1 or 2).
    pub channels: u16,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Output bits per sample (16 or 24).
    pub bits: u16,
    /// Frames written.
    pub frames_out: u64,
    /// Frames dropped from the start.
    pub trim_frames: u64,
    /// The source holds floats (its `fact` chunk is dropped).
    pub float_source: bool,
    /// What happens to the loudness fields of an existing `bext`.
    pub bext_update: BextUpdate,
}

impl Target {
    /// Bytes of audio written.
    pub fn audio_bytes(&self) -> u64 {
        self.frames_out * u64::from(self.channels) * u64::from(self.bits / 8)
    }
}

/// Where a chunk's payload comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Body {
    /// Copied from this byte range of the source.
    Copy(Range<u64>),
    /// These bytes (a new header or a patched payload).
    Bytes(Vec<u8>),
    /// The rendered audio (preceded by the zero offset and block size in `SSND`).
    Audio,
}

/// One output chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OutChunk {
    /// Chunk id.
    pub id: [u8; 4],
    /// Payload bytes (without the pad byte).
    pub len: u32,
    /// Payload source.
    pub body: Body,
    /// Pad byte after an odd payload: the source's value for carried and patched chunks, 0
    /// for new ones and where the source left it out.
    pub pad: Option<u8>,
}

/// The planned output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    /// Chunks in output order.
    pub chunks: Vec<OutChunk>,
    /// What happened to each source chunk, in source order.
    pub records: Vec<BlockRecord>,
    /// The container size field: bytes after the first 8, up to the end of the last chunk.
    pub form_size: u32,
    /// Bytes after the container copied from the source (stray bytes, an `ID3v1` tag).
    pub trailing: Option<Range<u64>>,
    /// Length of the output file, bytes.
    pub total_bytes: u64,
}

/// Plans the output of `format` in `table` for `target`.
///
/// # Errors
/// [`Error::UnsupportedFormat`] for a second format or audio chunk or a chunk to patch above
/// [`MAX_PATCHED_CHUNK_BYTES`] or chunks to patch above [`MAX_PATCHED_TOTAL_BYTES`] together;
/// [`Error::Corrupt`] for a malformed position chunk; [`Error::InvalidArgument`] when the trim
/// would cut a sampler loop; [`Error::NotDjSafe`] when the output would pass 4 GiB (WAV) or
/// [`MAX_AIFF_FORM_SIZE`] (AIFF); [`Error::Io`] when reading fails.
pub(super) fn plan<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    table: &ChunkTable,
    format: &AudioFormat,
    target: &Target,
) -> Result<Layout> {
    let wave = table.container.is_wave();
    let mut chunks = Vec::with_capacity(table.chunks.len());
    let mut records = Vec::with_capacity(table.chunks.len());
    let mut held = 0_u64;
    for (i, chunk) in table.chunks.iter().enumerate() {
        let decision = if i == format.format_chunk {
            Decision::Replace(format_payload(target))
        } else if Some(i) == format.audio_chunk {
            Decision::Audio
        } else {
            decide(src, path, wave, chunk, target, &mut held)?
        };
        let (fate, out) = match decision {
            Decision::Drop => (BlockFate::Dropped, None),
            Decision::Carry => (BlockFate::Carried, Some(carried(path, chunk)?)),
            Decision::Patch(bytes) => (
                BlockFate::Patched,
                Some(new_chunk(path, chunk.id, bytes, chunk_pad(chunk))?),
            ),
            Decision::Replace(bytes) => (
                BlockFate::Replaced,
                Some(new_chunk(path, chunk.id, bytes, Some(0))?),
            ),
            Decision::Audio => (
                BlockFate::Replaced,
                Some(audio_chunk(path, chunk.id, wave, target)?),
            ),
        };
        records.push(BlockRecord {
            id: chunk.id,
            fate,
            source_bytes: chunk.payload_len(),
        });
        chunks.extend(out);
    }
    let body: u64 = chunks
        .iter()
        .map(|c| 8 + u64::from(c.len) + u64::from(c.pad.is_some()))
        .sum();
    let limit = if wave {
        u64::from(u32::MAX)
    } else {
        MAX_AIFF_FORM_SIZE
    };
    let form_size = u32::try_from(4 + body)
        .ok()
        .filter(|size| u64::from(*size) <= limit)
        .ok_or_else(|| Error::NotDjSafe {
            path: path.to_path_buf(),
            reason: format!(
                "the output would hold {} bytes, past the {} a {} file can address",
                12 + body,
                if wave { "4 GiB" } else { "2 GiB" },
                if wave { "RIFF/WAVE" } else { "FORM/AIFF" }
            ),
        })?;
    let trailing = table.trailing.clone();
    let trailing_len = trailing.as_ref().map_or(0, |t| t.end - t.start);
    Ok(Layout {
        chunks,
        records,
        form_size,
        trailing,
        total_bytes: 8 + u64::from(form_size) + trailing_len,
    })
}

/// What happens to one source chunk.
enum Decision {
    Drop,
    Carry,
    Patch(Vec<u8>),
    Replace(Vec<u8>),
    Audio,
}

fn format_payload(target: &Target) -> Vec<u8> {
    match target.container {
        OutContainer::RiffWave => {
            wave_fmt_pcm(target.channels, target.sample_rate, target.bits).to_vec()
        }
        // frames_out fits 32 bits: the audio fits a container below 4 GiB, checked after.
        OutContainer::FormAiff => aiff_comm(
            target.channels,
            u32::try_from(target.frames_out).unwrap_or(u32::MAX),
            target.bits,
            target.sample_rate,
        )
        .to_vec(),
    }
}

/// The fate of a chunk that is neither the format nor the audio chunk.
fn decide<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    wave: bool,
    chunk: &Chunk,
    target: &Target,
    held: &mut u64,
) -> Result<Decision> {
    let trim = target.trim_frames;
    let id = &chunk.id;
    let duplicate = if wave {
        matches!(id, b"fmt " | b"data")
    } else {
        matches!(id, b"COMM" | b"SSND")
    };
    if duplicate {
        return Err(Error::UnsupportedFormat {
            path: path.to_path_buf(),
            detail: format!("a second '{}' chunk", chunk.id_text()),
        });
    }
    let rewrite = |src: &mut R, held: &mut u64, f: &dyn Fn(&mut [u8]) -> PatchResult| {
        let mut p = read_payload(src, path, chunk, held)?;
        match f(&mut p) {
            Ok(true) => Ok(Decision::Patch(p)),
            Ok(false) => {
                *held -= chunk.payload_len();
                Ok(Decision::Carry)
            }
            Err(e) => Err(patch_error(path, chunk, trim, &e)),
        }
    };
    let always = |r: std::result::Result<(), PatchError>| r.map(|()| true);
    match (wave, id) {
        (true, b"ds64") | (false, b"FVER") => Ok(Decision::Drop),
        (true, b"fact") if target.float_source => Ok(Decision::Drop),
        (true, b"fact") => rewrite(src, held, &|p| {
            always(patch::set_fact(p, target.frames_out))
        }),
        (true, b"cue ") if trim > 0 => rewrite(src, held, &|p| always(patch::shift_cue(p, trim))),
        (true, b"smpl") if trim > 0 => rewrite(src, held, &|p| always(patch::shift_smpl(p, trim))),
        (false, b"MARK") if trim > 0 => rewrite(src, held, &|p| always(patch::shift_mark(p, trim))),
        (true, b"bext") if trim > 0 || target.bext_update != BextUpdate::Keep => {
            rewrite(src, held, &|p| {
                patch::patch_bext(p, trim, target.bext_update)
            })
        }
        _ => Ok(Decision::Carry),
    }
}

/// A patch's outcome: whether the payload changed.
type PatchResult = std::result::Result<bool, PatchError>;

fn patch_error(path: &Path, chunk: &Chunk, trim: u64, e: &PatchError) -> Error {
    match *e {
        PatchError::Malformed(why) => Error::Corrupt {
            path: path.to_path_buf(),
            detail: format!(
                "'{}' chunk at byte {}: {why}",
                chunk.id_text(),
                chunk.header_offset
            ),
        },
        PatchError::LoopInTrim { start, end } => Error::InvalidArgument(format!(
            "{}: a head trim of {trim} samples would cut the sampler loop from sample {start} \
             to {end}",
            path.display()
        )),
    }
}

/// The pad byte of a carried or patched chunk: its own value, or 0 where the source left the
/// pad out.
fn chunk_pad(chunk: &Chunk) -> Option<u8> {
    match chunk.pad {
        Some(v) => Some(v),
        None if chunk.payload_len() % 2 == 1 => Some(0),
        None => None,
    }
}

fn too_large(path: &Path, what: &str) -> Error {
    Error::NotDjSafe {
        path: path.to_path_buf(),
        reason: format!("{what} is larger than a 32-bit chunk size can hold"),
    }
}

fn carried(path: &Path, chunk: &Chunk) -> Result<OutChunk> {
    let len = u32::try_from(chunk.payload_len())
        .map_err(|_| too_large(path, &format!("the '{}' chunk", chunk.id_text())))?;
    Ok(OutChunk {
        id: chunk.id,
        len,
        body: Body::Copy(chunk.payload.clone()),
        pad: chunk_pad(chunk),
    })
}

fn new_chunk(path: &Path, id: [u8; 4], bytes: Vec<u8>, pad_value: Option<u8>) -> Result<OutChunk> {
    let len = u32::try_from(bytes.len()).map_err(|_| too_large(path, "a chunk"))?;
    Ok(OutChunk {
        id,
        len,
        body: Body::Bytes(bytes),
        pad: (len % 2 == 1).then_some(pad_value.unwrap_or(0)),
    })
}

fn audio_chunk(path: &Path, id: [u8; 4], wave: bool, target: &Target) -> Result<OutChunk> {
    let fields = if wave {
        0
    } else {
        u64::from(SSND_FIELDS_BYTES)
    };
    let len =
        u32::try_from(fields + target.audio_bytes()).map_err(|_| too_large(path, "the audio"))?;
    Ok(OutChunk {
        id,
        len,
        body: Body::Audio,
        pad: (len % 2 == 1).then_some(0),
    })
}

/// Reads a chunk's payload to patch it, counting it in `held`.
fn read_payload<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    chunk: &Chunk,
    held: &mut u64,
) -> Result<Vec<u8>> {
    let len = chunk.payload_len();
    if len > MAX_PATCHED_CHUNK_BYTES {
        return Err(Error::UnsupportedFormat {
            path: path.to_path_buf(),
            detail: format!(
                "a '{}' chunk of {len} bytes is too large to rewrite (at most {MAX_PATCHED_CHUNK_BYTES})",
                chunk.id_text()
            ),
        });
    }
    if *held + len > MAX_PATCHED_TOTAL_BYTES {
        return Err(Error::UnsupportedFormat {
            path: path.to_path_buf(),
            detail: format!(
                "the chunks to rewrite hold more than {MAX_PATCHED_TOTAL_BYTES} bytes together \
                 ('{}' would bring them to {})",
                chunk.id_text(),
                *held + len
            ),
        });
    }
    *held += len;
    let mut p = vec![0; usize::try_from(len).unwrap_or(0)];
    src.seek(SeekFrom::Start(chunk.payload.start))
        .and_then(|_| src.read_exact(&mut p))
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(p)
}

#[cfg(test)]
mod tests;
