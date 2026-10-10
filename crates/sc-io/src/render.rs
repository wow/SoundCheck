//! Rendering a WAV, RF64, AIFF or AIFF-C file with a new level (and an optional head trim) into
//! a DJ-safe file of the same family, losing nothing else.
//!
//! [`apply_iff`] streams `source -> trim -> fade-in -> gain -> requantise -> writer`, with
//! memory bounded by one block of audio whatever the file length:
//!
//! - **Head cut** ([`RenderRequest::trim_frames`]): moved earlier by up to 1 ms to the quietest
//!   frame, never later, and the first 2 ms after it faded in (see [`head`]); every position
//!   shift below uses the cut actually made, which [`RenderReport::trim_frames`] reports next to
//!   the requested one.
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
//!   five fields of a version 2 `bext` become 7FFFh ("not measured") rather than stay stale
//!   (versions 0 and 1 have none and are kept). A `bext` that needs no change is carried as it
//!   is, even when it is shorter than the 602 bytes the specification requires.
//! - **Dropped**: `ds64` (the output is RIFF), AIFF-C `FVER` (the output is AIFF), and the
//!   `fact` of a float source (it describes non-PCM data).
//! - **Carried byte for byte**, in source order with the source's pad byte value: everything
//!   else (`LIST`, `id3 `/`ID3 `, iXML, `JUNK`, `APPL`, unknown chunks, chunks after the
//!   audio) and the bytes after the container (an `ID3v1` tag) at the end of the file.
//! - **Tag edits** ([`RenderRequest::tag_edits`]): the one `id3 `/`ID3 ` chunk keeps its id and
//!   place and gets SoundCheck's text frames (replaced in place where a frame with the same
//!   label exists, else appended; every other frame byte for byte; see [`crate::id3`]). With no
//!   ID3 chunk (none is created), more than one (readers disagree on which counts), or a tag
//!   that cannot be edited safely, every chunk is carried, the render goes on, and
//!   [`RenderReport::tags_not_added`] says why.
//!
//! Refused, with no output file left behind: more than two channels
//! ([`Error::UnsupportedChannels`]); a sample rate other than 44,100 or 48,000 Hz, or an output
//! past 4 GiB (WAV) or 2 GiB (AIFF, whose sizes are signed) ([`Error::NotDjSafe`]); a sample
//! after gain at or above +1.0 or below -1.0 of full scale ([`Error::WouldClip`]: never
//! clipped; -1.0 itself is a valid code; checked by a first pass over the samples for a float
//! source and for a boost); data in the container's padding bits, a second format or audio
//! chunk, more than 16 MiB of chunks to rewrite ([`Error::UnsupportedFormat`]); an invalid
//! tag edit (a label other than a text frame id or `TXXX:<description>`, a NUL, a label
//! given twice) ([`Error::InvalidArgument`]); a
//! trim that leaves no audio or starts inside a sampler loop ([`Error::InvalidArgument`]); a
//! file cut short inside a chunk, a malformed chunk that must be rewritten
//! ([`Error::Corrupt`]). A cancel flag, checked once per block, stops the render with
//! [`Error::Cancelled`] and the partial output removed (as on any error or panic). After a gain
//! change, ID3 `TXXX:REPLAYGAIN_*` and `RVA2` frames that no edit replaces are listed in
//! [`RenderReport::stale_loudness_tags`]. The output is created new
//! (`create_new`), so an existing file, the source included, is never overwritten; making the
//! write atomic is the caller's job. I/O errors name the input when reading fails and the
//! output when writing fails.

mod audio;
mod flac;
pub mod head;
mod layout;
pub mod patch;
mod tag;

pub(crate) use audio::check_cancel;
pub use flac::apply_flac;
pub(crate) use flac::{FlacCheck, render_flac_unverified};
pub use head::{HEAD_FADE_MS, HEAD_SNAP_MAX_MS, head_fade_frames, head_snap_frames};

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, Result};
use sc_dsp::db_to_linear;

use crate::id3::{self, EditSummary, NotEditable};
use crate::iff::{self, AudioFormat, ChunkTable, OutContainer, chunk_header, container_header};
use audio::{AudioJob, SeedRequest};
use layout::{Body, Layout, Target};
use patch::BextUpdate;
use tag::TagOutcome;

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
    /// A tag (an ID3 tag, a FLAC Vorbis comment) with SoundCheck's items written and every
    /// other item copied.
    Edited,
    /// Written anew (format and audio chunks; FLAC STREAMINFO, SEEKTABLE and PADDING).
    Replaced,
    /// Left out of the output.
    Dropped,
}

/// What a source block is: a WAV/AIFF chunk or a FLAC metadata block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockId {
    /// A chunk with this four-character id.
    Chunk([u8; 4]),
    /// A FLAC metadata block of this type (RFC 9639 table 2: 0 STREAMINFO, 1 PADDING, ...).
    FlacBlock(u8),
}

/// One source chunk or metadata block and its fate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockRecord {
    /// Chunk id or block type.
    pub id: BlockId,
    /// What happened to it.
    pub fate: BlockFate,
    /// Its payload length in the source, bytes.
    pub source_bytes: u64,
}

/// What [`apply_iff`] or [`apply_flac`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderReport {
    /// Whether the requested tag edits were written.
    pub tags_added: bool,
    /// Why requested tag edits were not written (the tag, if any, is carried unchanged);
    /// `None` when they were written or none were requested.
    pub tags_not_added: Option<NotEditable>,
    /// What the ID3 tag edit did, when it was written (WAV/AIFF).
    pub tag_edit: Option<EditSummary>,
    /// What the Vorbis comment edit did, when it was written (FLAC).
    pub vorbis_edit: Option<crate::flac::vorbis::EditSummary>,
    /// Frames in the source.
    pub frames_in: u64,
    /// Frames written: `frames_in - trim_frames`.
    pub frames_out: u64,
    /// Frames cut from the start: the requested cut moved earlier by up to 1 ms to the
    /// quietest frame (see [`head`]); 0 when none was requested. Positions in the output are
    /// the source's minus this.
    pub trim_frames: u64,
    /// Frames the request asked to cut ([`RenderRequest::trim_frames`]).
    pub trim_requested_frames: u64,
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
    /// BLAKE3 of the audio as written: the `data` payload, or the sound data after the
    /// `SSND` offset and block size; for FLAC the samples as its MD5 signature covers them
    /// (interleaved, little-endian, `bits_out / 8` bytes each), the bytes a decode of the
    /// frames gives.
    pub pcm_hash: [u8; 32],
    /// Loudness tags the render leaves stale, by label as written: after a gain change, the
    /// Replay Gain and R 128 items no tag edit replaces (Vorbis `REPLAYGAIN_*` and `R128_*`
    /// fields; ID3 `TXXX:REPLAYGAIN_*` and `RVA2` frames). Empty without a gain change. They
    /// are carried unchanged; the caller decides whether to edit them.
    pub stale_loudness_tags: Vec<String>,
    /// Every source chunk or metadata block in source order with its fate.
    pub blocks: Vec<BlockRecord>,
    /// Bytes before the stream carried at the start of the output (`ID3v2` tags in front of
    /// `fLaC`; 0 for WAV/AIFF).
    pub leading_bytes: u64,
    /// Bytes after the container or the last FLAC frame carried at the end of the output.
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

/// Removes a newly created output file when dropped, unless [`OutputGuard::keep`] was called:
/// an error or a panic while writing never leaves a partial file behind.
#[derive(Debug)]
pub(crate) struct OutputGuard<'a> {
    path: &'a Path,
    keep: bool,
}

impl<'a> OutputGuard<'a> {
    /// Guards the file at `path`, which the caller has just created.
    pub(crate) fn new(path: &'a Path) -> Self {
        crate::txn::crash::point("render");
        Self { path, keep: false }
    }

    /// Keeps the file: it is complete.
    pub(crate) fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for OutputGuard<'_> {
    fn drop(&mut self) {
        if !self.keep
            && let Err(rm) = std::fs::remove_file(self.path)
        {
            tracing::warn!(path = %self.path.display(), error = %rm, "partial output not removed");
        }
    }
}

fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Renders `input` with `req` applied into the new file `output`; `cancel` (checked once per
/// block of audio) stops it.
///
/// # Errors
/// The refusals listed in the module documentation, [`Error::InvalidArgument`] for a request
/// that is not valid (gain not finite, depth other than 16 or 24), [`Error::Cancelled`],
/// [`Error::Io`] when reading or writing fails or `output` exists. On any error no output file
/// is left.
pub fn apply_iff(
    input: &Path,
    output: &Path,
    req: &RenderRequest,
    cancel: &AtomicBool,
) -> Result<RenderReport> {
    check_request(req)?;
    let tag_edits = id3::edits_from(&req.tag_edits)?;
    let mut src = File::open(input).map_err(|e| io_error(input, e))?;
    let header = iff::read_header(&mut src, input)?;
    let (table, format) = (&header.table, &header.format);
    let mut target = target(input, table, format, req, tag_edits)?;
    target.trim_frames = audio::snapped_trim(&mut src, input, format, req.trim_frames, cancel)?;
    target.frames_out = format.frames - target.trim_frames;
    let layout = layout::plan(&mut src, input, table, format, &target)?;
    tracing::debug!(
        path = %input.display(),
        stage = "render-plan",
        chunks = layout.chunks.len(),
        output_bytes = layout.total_bytes,
        "planned"
    );
    if format.encoding.is_float() || req.gain_db > 0.0 {
        let peaks = audio::peaks_after_trim(&mut src, input, format, target.trim_frames, cancel)?;
        check_full_scale(peaks, req.gain_db)?;
    }
    let stale_loudness_tags = if req.gain_db == 0.0 {
        Vec::new()
    } else {
        tag::stale_loudness(&mut src, input, table, &target.tag_edits)?
    };
    let format_payload = read_format_payload(&mut src, input, table, format)?;
    let seed_request = SeedRequest {
        gain_db: req.gain_db,
        trim_frames: target.trim_frames,
        bits: target.bits,
    };
    let seed = audio::dither_seed(
        &mut src,
        input,
        format,
        &format_payload,
        seed_request,
        cancel,
    )?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| io_error(output, e))?;
    let job = AudioJob {
        input,
        output,
        format,
        trim_frames: target.trim_frames,
        frames_out: target.frames_out,
        bits: target.bits,
        big_endian: target.container.is_big_endian(),
        gain_db: req.gain_db,
        seed,
        cancel,
    };
    let guard = OutputGuard::new(output);
    let done = write_file(&mut src, file, output, &layout, target.container, &job)?;
    guard.keep();
    tracing::info!(
        path = %input.display(),
        output = %output.display(),
        gain_db = req.gain_db,
        trim_frames = target.trim_frames,
        trim_requested_frames = req.trim_frames,
        bits = target.bits,
        frames = target.frames_out,
        bytes = layout.total_bytes,
        tags_added = matches!(layout.tags, TagOutcome::Edited(_)),
        "rendered"
    );
    let (tag_edit, tags_not_added) = match &layout.tags {
        TagOutcome::Edited(summary) => (Some(*summary), None),
        TagOutcome::NotAdded(reason) => (None, Some(reason.clone())),
        TagOutcome::NotRequested | TagOutcome::Pending(_) => (None, None),
    };
    Ok(RenderReport {
        tags_added: tag_edit.is_some(),
        tags_not_added,
        tag_edit,
        vorbis_edit: None,
        frames_in: format.frames,
        frames_out: target.frames_out,
        trim_frames: target.trim_frames,
        trim_requested_frames: req.trim_frames,
        sample_rate_hz: format.sample_rate,
        channels: format.channels,
        bits_out: target.bits,
        exact: done.exact,
        dithered: done.dithered,
        samples_saturated: done.saturated,
        pcm_hash: done.pcm_hash,
        stale_loudness_tags,
        blocks: layout.records,
        leading_bytes: 0,
        trailing_bytes: layout.trailing.map_or(0, |t| t.end - t.start),
        output_bytes: layout.total_bytes,
    })
}

/// Refuses a gain that takes a sample to +1.0 of full scale or beyond, or below -1.0 (the most
/// negative code is exactly -1.0, so it may stay).
fn check_full_scale(peaks: audio::Peaks, gain_db: f64) -> Result<()> {
    let gain = if gain_db == 0.0 {
        1.0
    } else {
        db_to_linear(gain_db)
    };
    let (max, min) = (peaks.max * gain, peaks.min * gain);
    if max >= 1.0 || min < -1.0 {
        return Err(Error::WouldClip {
            needed_db: gain_db,
            over_db: 20.0 * max.max(-min).log10(),
        });
    }
    Ok(())
}

/// Checks the request's gain and depth (tag edits are checked by each container's rules).
fn check_request(req: &RenderRequest) -> Result<()> {
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
    Ok(())
}

/// The output depth: the requested one, else 24 for a float source or one deeper than 16
/// bits, 16 otherwise.
fn output_bits(req: &RenderRequest, float_source: bool, source_bits: u16) -> u16 {
    match req.bits {
        Some(b) => u16::from(b),
        None if float_source || source_bits > 16 => 24,
        None => 16,
    }
}

/// [`Error::NotDjSafe`] unless the rate is one of [`DJ_SAFE_RATES_HZ`].
fn check_rate(path: &Path, sample_rate_hz: u32) -> Result<()> {
    if DJ_SAFE_RATES_HZ.contains(&sample_rate_hz) {
        return Ok(());
    }
    Err(Error::NotDjSafe {
        path: path.to_path_buf(),
        reason: format!(
            "sample rate {sample_rate_hz} Hz; DJ-safe outputs are 44,100 or 48,000 Hz and \
             SoundCheck does not convert rates"
        ),
    })
}

/// The output's shape, or why the source cannot be rendered DJ-safe.
fn target(
    path: &Path,
    table: &ChunkTable,
    format: &AudioFormat,
    req: &RenderRequest,
    tag_edits: Vec<id3::Edit>,
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
    check_rate(path, format.sample_rate)?;
    if req.trim_frames >= format.frames {
        return Err(Error::InvalidArgument(format!(
            "{}: a head trim of {} samples leaves no audio ({} frames)",
            path.display(),
            req.trim_frames,
            format.frames
        )));
    }
    let float_source = format.encoding.is_float();
    let bits = output_bits(req, float_source, format.valid_bits);
    let bext_update = match (req.loudness, req.gain_db != 0.0) {
        (Some(l), _) => BextUpdate::Set(l.to_le_bytes()),
        (None, true) => BextUpdate::ClearIfVersion2,
        (None, false) => BextUpdate::Keep,
    };
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
        bext_update,
        tag_edits,
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

/// Bytes copied per read when carrying a byte range.
const COPY_BUFFER_BYTES: usize = 64 << 10;

/// Copies `range` of `src` (the file `input`) into `w` (the file `output`); read errors name
/// the input, write errors the output.
fn copy_range<R: Read + Seek, W: Write>(
    src: &mut R,
    input: &Path,
    output: &Path,
    range: &std::ops::Range<u64>,
    w: &mut W,
) -> Result<()> {
    src.seek(SeekFrom::Start(range.start))
        .map_err(|e| io_error(input, e))?;
    let mut buf = vec![0_u8; COPY_BUFFER_BYTES];
    let mut left = range.end - range.start;
    while left > 0 {
        let want = usize::try_from(left).map_or(buf.len(), |l| l.min(buf.len()));
        let got = match src.read(&mut buf[..want]) {
            Ok(0) => {
                return Err(Error::Corrupt {
                    path: input.to_path_buf(),
                    detail: format!(
                        "the file ends {left} bytes before the end of the range at byte {} \
                         (the file changed?)",
                        range.start
                    ),
                });
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error(input, e)),
        };
        w.write_all(&buf[..got]).map_err(|e| io_error(output, e))?;
        left -= got as u64;
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
    let input = job.input;
    let out_err = |e| io_error(output, e);
    let mut w = BufWriter::with_capacity(WRITE_BUFFER_BYTES, file);
    w.write_all(&container_header(container, layout.form_size))
        .map_err(out_err)?;
    let mut done = None;
    for chunk in &layout.chunks {
        w.write_all(&chunk_header(container, chunk.id, chunk.len))
            .map_err(out_err)?;
        match &chunk.body {
            Body::Copy(range) => copy_range(src, input, output, range, &mut w)?,
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
        copy_range(src, input, output, range, &mut w)?;
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
