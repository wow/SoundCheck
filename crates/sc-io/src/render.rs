//! Rendering a WAV, RF64, AIFF or AIFF-C file with a new level (and an optional head trim) into
//! a DJ-safe file of the same family, losing nothing else.
//!
//! [`apply_iff`] streams `source -> trim -> gain -> requantise -> writer`, with memory bounded
//! by one block of audio whatever the file length:
//!
//! - **Container**: WAV and RF64 (EBU Tech 3306) become `RIFF`/`WAVE`; AIFF and AIFF-C become
//!   `FORM`/`AIFF` (AIFF 1.3). The container size is recomputed; chunks a stale size left
//!   outside the container are carried inside it.
//! - **Format**: `fmt ` is replaced by a 16-byte `WAVE_FORMAT_PCM` header (never
//!   `WAVE_FORMAT_EXTENSIBLE`, also for an extensible or float source), `COMM` by an 18-byte
//!   AIFF `COMM`; channels and sample rate are kept.
//! - **Audio**: `data`/`SSND` is replaced in place by the rendered samples (`SSND` offset and
//!   block size 0), at the requested depth (16 or 24 bits) or the source's (a float source or
//!   one deeper than 24 bits gives 24, one of 16 bits or fewer gives 16); see
//!   [`sc_dsp::requantise`] for the exact, rounded and dithered cases.
//! - **Patched in place** (every other byte kept, the pad byte value too): `cue ` and `smpl`
//!   positions and AIFF `MARK` positions minus the trim, clamped at 0; a PCM `fact` takes the
//!   new length; `bext` (EBU Tech 3285 v2) gets `TimeReference` plus the trim and, when
//!   loudness is given, `Version` 2 and its loudness fields; after a gain without loudness the
//!   five fields become 7FFFh ("not measured") rather than stay stale.
//! - **Dropped**: `ds64` (the output is RIFF), AIFF-C `FVER` (the output is AIFF), and the
//!   `fact` of a float source (it describes non-PCM data).
//! - **Carried byte for byte**, in source order with the source's pad byte value: everything
//!   else (`LIST`, `id3 `/`ID3 `, iXML, `JUNK`, `APPL`, unknown chunks, chunks after the
//!   audio) and the bytes after the container (an `ID3v1` tag) at the end of the file.
//!
//! Refused, with no output file left behind: more than two channels
//! ([`Error::UnsupportedChannels`]); a sample rate other than 44,100 or 48,000 Hz, or an output
//! past 4 GiB ([`Error::NotDjSafe`]); a peak after gain at or above full scale
//! ([`Error::WouldClip`]: never clipped; checked by a first pass over the samples for a float
//! source and for a boost); data in the container's padding bits, a second format or audio
//! chunk, tag edits ([`Error::UnsupportedFormat`]); a trim that leaves no audio or would cut a
//! sampler loop ([`Error::InvalidArgument`]); a file cut short inside a chunk
//! ([`Error::Corrupt`]). The output is created new (`create_new`), so an existing file, the
//! source included, is never overwritten; making the write atomic is the caller's job.

mod audio;
mod layout;
pub mod patch;

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use sc_core::{BextLoudness, Error, RenderRequest, Result};
use sc_dsp::db_to_linear;

use crate::iff::{self, AudioFormat, ChunkTable, OutContainer, chunk_header, container_header};
use audio::AudioJob;
use layout::{Body, Layout, Target};

/// Sample rates a DJ-safe output may have, Hz.
pub const DJ_SAFE_RATES_HZ: [u32; 2] = [44_100, 48_000];

/// Output buffer size, bytes.
const WRITE_BUFFER_BYTES: usize = 1 << 20;

/// Largest format chunk payload hashed into the dither seed.
const MAX_SEED_FORMAT_BYTES: u64 = 4096;

/// What happened to a source chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockFate {
    /// Copied byte for byte.
    Carried,
    /// Copied with position or loudness fields rewritten.
    Patched,
    /// Written anew (format and audio chunks).
    Replaced,
    /// Left out of the output.
    Dropped,
}

/// One source chunk and its fate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockRecord {
    /// Chunk id.
    pub id: [u8; 4],
    /// What happened to it.
    pub fate: BlockFate,
    /// Its payload length in the source, bytes.
    pub source_bytes: u64,
}

/// What [`apply_iff`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderReport {
    /// Whether tag edits were written (always false until tag editing exists; edits are
    /// refused instead).
    pub tags_added: bool,
    /// Frames in the source.
    pub frames_in: u64,
    /// Frames written.
    pub frames_out: u64,
    /// Sample rate, Hz (unchanged).
    pub sample_rate_hz: u32,
    /// Channels (unchanged).
    pub channels: u16,
    /// Output bits per sample.
    pub bits_out: u16,
    /// Whether the samples are an exact copy or shift of the source's (0 dB, no lost depth).
    pub exact: bool,
    /// Whether TPDF dither was added.
    pub dithered: bool,
    /// Samples moved to the largest or smallest code by less than one step of rounding or
    /// dither (never a clip: those inputs are refused).
    pub samples_saturated: u64,
    /// BLAKE3 of the audio bytes exactly as written (the `data` payload, or the sound data
    /// after the `SSND` offset and block size).
    pub pcm_hash: [u8; 32],
    /// Every source chunk in source order with its fate.
    pub blocks: Vec<BlockRecord>,
    /// Bytes after the container carried at the end of the output.
    pub trailing_bytes: u64,
    /// Length of the output file, bytes.
    pub output_bytes: u64,
}

impl RenderReport {
    /// Number of source chunks with `fate`.
    #[must_use]
    pub fn count(&self, fate: BlockFate) -> usize {
        self.blocks.iter().filter(|b| b.fate == fate).count()
    }
}

fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Renders `input` with `req` applied into the new file `output`.
///
/// # Errors
/// The refusals listed in the module documentation, [`Error::InvalidArgument`] for a request
/// that is not valid (gain not finite, depth other than 16 or 24), [`Error::Io`] when reading
/// or writing fails or `output` exists. On any error no output file is left.
pub fn apply_iff(input: &Path, output: &Path, req: &RenderRequest) -> Result<RenderReport> {
    check_request(input, req)?;
    let mut src = File::open(input).map_err(|e| io_error(input, e))?;
    let header = iff::read_header(&mut src, input)?;
    let (table, format) = (&header.table, &header.format);
    let target = target(input, table, format, req)?;
    let layout = layout::plan(&mut src, input, table, format, &target)?;
    tracing::debug!(
        path = %input.display(),
        stage = "render-plan",
        chunks = layout.chunks.len(),
        output_bytes = layout.total_bytes,
        "planned"
    );
    if format.encoding.is_float() || req.gain_db > 0.0 {
        let peak = audio::peak_after_trim(&mut src, input, format, req.trim_frames)?;
        let after = peak * gain_linear(req.gain_db);
        if after >= 1.0 {
            return Err(Error::WouldClip {
                needed_db: req.gain_db,
                over_db: 20.0 * after.log10(),
            });
        }
    }
    let format_payload = read_format_payload(&mut src, input, table, format)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| io_error(output, e))?;
    let job = AudioJob {
        path: input,
        format,
        trim_frames: req.trim_frames,
        frames_out: target.frames_out,
        bits: target.bits,
        big_endian: target.container.is_big_endian(),
        gain_db: req.gain_db,
        format_payload: &format_payload,
    };
    let written = write_file(&mut src, file, output, &layout, target.container, &job);
    let done = match written {
        Ok(done) => done,
        Err(e) => {
            if let Err(rm) = std::fs::remove_file(output) {
                tracing::warn!(path = %output.display(), error = %rm, "partial output not removed");
            }
            return Err(e);
        }
    };
    tracing::info!(
        path = %input.display(),
        output = %output.display(),
        gain_db = req.gain_db,
        trim_frames = req.trim_frames,
        bits = target.bits,
        frames = target.frames_out,
        bytes = layout.total_bytes,
        "rendered"
    );
    Ok(RenderReport {
        tags_added: false,
        frames_in: format.frames,
        frames_out: target.frames_out,
        sample_rate_hz: format.sample_rate,
        channels: format.channels,
        bits_out: target.bits,
        exact: done.exact,
        dithered: done.dithered,
        samples_saturated: done.saturated,
        pcm_hash: done.pcm_hash,
        blocks: layout.records,
        trailing_bytes: layout.trailing.map_or(0, |t| t.end - t.start),
        output_bytes: layout.total_bytes,
    })
}

fn gain_linear(gain_db: f64) -> f64 {
    if gain_db == 0.0 {
        1.0
    } else {
        db_to_linear(gain_db)
    }
}

fn check_request(input: &Path, req: &RenderRequest) -> Result<()> {
    if !req.gain_db.is_finite() {
        return Err(Error::InvalidArgument(format!(
            "gain {} dB is not finite",
            req.gain_db
        )));
    }
    if let Some(bits) = req.bits
        && bits != 16
        && bits != 24
    {
        return Err(Error::InvalidArgument(format!(
            "output depth {bits} bits; DJ-safe outputs have 16 or 24"
        )));
    }
    if !req.tag_edits.is_empty() {
        return Err(Error::UnsupportedFormat {
            path: input.to_path_buf(),
            detail: "tag edits in WAV and AIFF files are not supported yet".into(),
        });
    }
    Ok(())
}

/// The output's shape, or why the source cannot be rendered DJ-safe.
fn target(
    path: &Path,
    table: &ChunkTable,
    format: &AudioFormat,
    req: &RenderRequest,
) -> Result<Target> {
    if table.truncated {
        return Err(Error::Corrupt {
            path: path.to_path_buf(),
            detail: "the file ends inside a chunk".into(),
        });
    }
    if format.channels > 2 {
        return Err(Error::UnsupportedChannels {
            path: path.to_path_buf(),
            channels: usize::from(format.channels),
        });
    }
    if !DJ_SAFE_RATES_HZ.contains(&format.sample_rate) {
        return Err(Error::NotDjSafe {
            path: path.to_path_buf(),
            reason: format!(
                "sample rate {} Hz; DJ-safe outputs are 44,100 or 48,000 Hz and SoundCheck \
                 does not convert rates",
                format.sample_rate
            ),
        });
    }
    if req.trim_frames >= format.frames {
        return Err(Error::InvalidArgument(format!(
            "{}: a head trim of {} samples leaves no audio ({} frames)",
            path.display(),
            req.trim_frames,
            format.frames
        )));
    }
    let float_source = format.encoding.is_float();
    let bits = match req.bits {
        Some(b) => u16::from(b),
        None if float_source || format.valid_bits > 16 => 24,
        None => 16,
    };
    let bext_loudness = req
        .loudness
        .or((req.gain_db != 0.0).then_some(BextLoudness::ALL_UNMEASURED));
    Ok(Target {
        container: if table.container.is_wave() {
            OutContainer::RiffWave
        } else {
            OutContainer::FormAiff
        },
        channels: format.channels,
        sample_rate: format.sample_rate,
        bits,
        frames_out: format.frames - req.trim_frames,
        trim_frames: req.trim_frames,
        float_source,
        bext_loudness,
    })
}

/// The source's format chunk payload (at most [`MAX_SEED_FORMAT_BYTES`]).
fn read_format_payload<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    table: &ChunkTable,
    format: &AudioFormat,
) -> Result<Vec<u8>> {
    let chunk = &table.chunks[format.format_chunk];
    let len = chunk.payload_len().min(MAX_SEED_FORMAT_BYTES);
    let mut p = vec![0; usize::try_from(len).unwrap_or(0)];
    src.seek(SeekFrom::Start(chunk.payload.start))
        .and_then(|_| src.read_exact(&mut p))
        .map_err(|e| io_error(path, e))?;
    Ok(p)
}

/// Copies `range` of `src` into `w`.
fn copy_range<R: Read + Seek, W: Write>(
    src: &mut R,
    path: &Path,
    range: &std::ops::Range<u64>,
    w: &mut W,
) -> Result<()> {
    let len = range.end - range.start;
    src.seek(SeekFrom::Start(range.start))
        .map_err(|e| io_error(path, e))?;
    let copied = std::io::copy(&mut src.take(len), w).map_err(|e| io_error(path, e))?;
    if copied != len {
        return Err(Error::Corrupt {
            path: path.to_path_buf(),
            detail: format!(
                "{copied} of {len} bytes at byte {} could be read (the file changed?)",
                range.start
            ),
        });
    }
    Ok(())
}

/// Writes the planned output into `file`.
fn write_file(
    src: &mut File,
    file: File,
    output: &Path,
    layout: &Layout,
    container: OutContainer,
    job: &AudioJob<'_>,
) -> Result<audio::AudioDone> {
    let input = job.path;
    let out_err = |e| io_error(output, e);
    let mut w = BufWriter::with_capacity(WRITE_BUFFER_BYTES, file);
    w.write_all(&container_header(container, layout.form_size))
        .map_err(out_err)?;
    let mut done = None;
    for chunk in &layout.chunks {
        w.write_all(&chunk_header(container, chunk.id, chunk.len))
            .map_err(out_err)?;
        match &chunk.body {
            Body::Copy(range) => copy_range(src, input, range, &mut w)?,
            Body::Bytes(bytes) => w.write_all(bytes).map_err(out_err)?,
            Body::Audio => {
                if container == OutContainer::FormAiff {
                    w.write_all(&[0; iff::SSND_FIELDS_BYTES as usize])
                        .map_err(out_err)?;
                }
                done = Some(audio::write_audio(&mut *src, &mut w, job)?);
            }
        }
        if let Some(pad) = chunk.pad {
            w.write_all(&[pad]).map_err(out_err)?;
        }
    }
    if let Some(range) = &layout.trailing {
        copy_range(src, input, range, &mut w)?;
    }
    let file = w.into_inner().map_err(|e| out_err(e.into_error()))?;
    let len = file.metadata().map_err(out_err)?.len();
    if len != layout.total_bytes {
        return Err(Error::Internal(format!(
            "wrote {len} bytes to {}, planned {}",
            output.display(),
            layout.total_bytes
        )));
    }
    done.ok_or_else(|| Error::Internal("the plan held no audio chunk".into()))
}

#[cfg(test)]
mod tests;
