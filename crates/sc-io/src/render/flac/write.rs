//! Writing the planned FLAC output and verifying it.
//!
//! The leading tags, `fLaC` and the metadata blocks are written first, with STREAMINFO and
//! SEEKTABLE as zero bytes of their final length; then the frames, streamed from the source
//! through the requantiser and the encoder; then the bytes after the source's last frame. Once
//! the frames are known, STREAMINFO and every SEEKTABLE are written in place. Verification
//! walks the written file again and decodes its frames (see [`crate::flac::decode_frames`]).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::{Error, Result};
use sc_dsp::{Requantiser, SourceDepth};

use super::super::audio::check_cancel;
use super::super::{WRITE_BUFFER_BYTES, copy_range, io_error};
use super::Target;
use super::plan::{Body, Plan};
use crate::flac::seektable::{self, SeekShape};
use crate::flac::{
    Encoded, FlacLayout, FlacPcm, FrameEncoder, MARKER, OUTPUT_BLOCK_FRAMES, STREAMINFO_BYTES,
    StreamInfo, block_header, decode_frames, read_layout,
};

/// Zero bytes written per call for PADDING and placeholder payloads.
const ZEROS: [u8; 4096] = [0; 4096];

/// Everything the writer needs.
pub(super) struct Job<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub layout: &'a FlacLayout,
    pub plan: &'a Plan,
    pub target: &'a Target,
    pub cancel: &'a AtomicBool,
}

/// What was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Done {
    pub pcm_hash: [u8; 32],
    pub exact: bool,
    pub dithered: bool,
    pub saturated: u64,
    pub trailing_bytes: u64,
    pub output_bytes: u64,
}

fn write_zeros<W: Write>(w: &mut W, mut n: u64, output: &Path) -> Result<()> {
    while n > 0 {
        let k = usize::try_from(n).map_or(ZEROS.len(), |n| n.min(ZEROS.len()));
        w.write_all(&ZEROS[..k]).map_err(|e| io_error(output, e))?;
        n -= k as u64;
    }
    Ok(())
}

/// Writes the output into `file` and verifies it.
///
/// # Errors
/// Reading and writing errors, a source that does not decode as declared, an encoder guard,
/// [`Error::Cancelled`], and [`Error::Corrupt`] naming the output when verification fails.
pub(super) fn write_and_verify(src: &mut File, file: File, job: &Job<'_>) -> Result<Done> {
    let output = job.output;
    let out_err = |e| io_error(output, e);
    let mut w = BufWriter::with_capacity(WRITE_BUFFER_BYTES, file);
    copy_range(src, job.input, output, &job.layout.leading(), &mut w)?;
    w.write_all(&MARKER).map_err(out_err)?;
    let mut pos = job.layout.marker_offset + 4;
    let mut streaminfo_at = None;
    let mut tables: Vec<(u64, SeekShape)> = Vec::new();
    let count = job.plan.blocks.len();
    for (i, block) in job.plan.blocks.iter().enumerate() {
        w.write_all(&block_header(i + 1 == count, block.block_type, block.len))
            .map_err(out_err)?;
        pos += 4;
        match &block.body {
            Body::Copy(range) => copy_range(src, job.input, output, range, &mut w)?,
            Body::Bytes(bytes) => w.write_all(bytes).map_err(out_err)?,
            Body::Zeros(n) => write_zeros(&mut w, u64::from(*n), output)?,
            Body::StreamInfo => {
                streaminfo_at = Some(pos);
                write_zeros(&mut w, STREAMINFO_BYTES as u64, output)?;
            }
            Body::SeekTable(shape) => {
                tables.push((pos, *shape));
                write_zeros(&mut w, shape.bytes() as u64, output)?;
            }
        }
        pos += u64::from(block.len);
    }
    let frames_start = pos;
    let (encoded, audio) = write_frames(src, &mut w, job)?;
    let trailing = audio.frames_end..job.layout.file_len;
    copy_range(src, job.input, output, &trailing, &mut w)?;
    let mut file = w.into_inner().map_err(|e| out_err(e.into_error()))?;
    let target = job.target;
    // 4096 fits the 16-bit field.
    let block_u16 = u16::try_from(OUTPUT_BLOCK_FRAMES).unwrap_or(u16::MAX);
    let info = StreamInfo {
        min_block: block_u16,
        max_block: block_u16,
        min_frame: encoded.min_frame,
        max_frame: encoded.max_frame,
        sample_rate_hz: job.layout.streaminfo.sample_rate_hz,
        channels: job.layout.streaminfo.channels,
        // 16 or 24.
        bits: u8::try_from(target.bits_out).unwrap_or(24),
        total_samples: encoded.total_samples,
        md5: encoded.md5,
    };
    let at = streaminfo_at.ok_or_else(|| Error::Internal("the plan held no STREAMINFO".into()))?;
    let mut patch = |at: u64, bytes: &[u8]| {
        file.seek(SeekFrom::Start(at))
            .and_then(|_| file.write_all(bytes))
            .map_err(out_err)
    };
    patch(at, &info.to_bytes())?;
    for (at, shape) in &tables {
        let table = seektable::build(
            *shape,
            &encoded.frame_offsets,
            OUTPUT_BLOCK_FRAMES as u64,
            encoded.total_samples,
        );
        patch(*at, &table)?;
    }
    let output_bytes = file.metadata().map_err(out_err)?.len();
    let planned = job.layout.marker_offset
        + job.plan.metadata_bytes()
        + encoded.frames_bytes
        + (trailing.end - trailing.start);
    if output_bytes != planned {
        return Err(Error::Internal(format!(
            "wrote {output_bytes} bytes to {}, planned {planned}",
            output.display()
        )));
    }
    drop(file);
    let want = Expected {
        block_types: job.plan.blocks.iter().map(|b| b.block_type).collect(),
        frames_start,
        info,
        frames_bytes: encoded.frames_bytes,
        pcm_hash: encoded.pcm_hash,
    };
    verify(output, &want, job.cancel)?;
    Ok(Done {
        pcm_hash: encoded.pcm_hash,
        trailing_bytes: trailing.end - trailing.start,
        output_bytes,
        ..audio.done
    })
}

/// What the audio stage did besides the frames.
struct Audio {
    done: Done,
    frames_end: u64,
}

/// Streams the source's samples through trim, gain, requantisation and the encoder into `w`.
fn write_frames<W: Write>(src: &mut File, w: &mut W, job: &Job<'_>) -> Result<(Encoded, Audio)> {
    let target = job.target;
    let info = &job.layout.streaminfo;
    let channels = usize::from(info.channels);
    let mut quant = Requantiser::new(
        SourceDepth::Int {
            bits: target.bits_in,
        },
        target.bits_out,
        target.gain_db,
        target.seed,
    )?;
    let mut flac_encoder = FrameEncoder::new(
        info.sample_rate_hz,
        u16::from(info.channels),
        target.bits_out,
        job.output,
    )?;
    let mut pcm = FlacPcm::open(&mut *src, job.input, job.layout, true)?;
    let (mut block, mut out) = (Vec::new(), Vec::new());
    let mut skip = target.trim_frames.saturating_mul(channels as u64);
    loop {
        check_cancel(job.cancel)?;
        if pcm.next_block(&mut block)? == 0 {
            break;
        }
        let start = usize::try_from(skip.min(block.len() as u64)).unwrap_or(0);
        skip -= start as u64;
        let kept = block.len() - start;
        if out.len() < kept {
            out.resize(kept, 0);
        }
        quant.push_int(&block[start..], &mut out[..kept])?;
        flac_encoder.push(&out[..kept], w)?;
    }
    let read = pcm.finish()?;
    if read.frames != target.frames_in {
        return Err(Error::Corrupt {
            path: job.input.to_path_buf(),
            detail: format!(
                "{} frames of audio read, {} expected (the file changed?)",
                read.frames, target.frames_in
            ),
        });
    }
    let encoded = flac_encoder.finish(w)?;
    if encoded.total_samples != target.frames_out {
        return Err(Error::Internal(format!(
            "encoded {} frames, planned {}",
            encoded.total_samples, target.frames_out
        )));
    }
    let done = Done {
        pcm_hash: encoded.pcm_hash,
        exact: quant.is_exact(),
        dithered: quant.is_dithered(),
        saturated: quant.samples_saturated(),
        trailing_bytes: 0,
        output_bytes: 0,
    };
    Ok((
        encoded,
        Audio {
            done,
            frames_end: read.frames_end,
        },
    ))
}

fn verify_failed(output: &Path, what: &str) -> Error {
    Error::Corrupt {
        path: output.to_path_buf(),
        detail: format!("verification of the written file failed: {what}"),
    }
}

/// What verification expects of the written file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Expected {
    /// Metadata block types in order.
    pub block_types: Vec<u8>,
    /// Byte offset of the first frame.
    pub frames_start: u64,
    /// The STREAMINFO written.
    pub info: StreamInfo,
    /// Length of the frames, bytes.
    pub frames_bytes: u64,
    /// BLAKE3 of the samples encoded.
    pub pcm_hash: [u8; 32],
}

/// Walks the written file `output` and decodes its frames: the metadata must be where it was
/// planned with the STREAMINFO written, and the frames must decode to the encoded samples
/// (count and BLAKE3) with the STREAMINFO MD5.
///
/// # Errors
/// [`Error::Corrupt`] naming `output` for any difference; [`Error::Io`]; [`Error::Cancelled`].
pub(super) fn verify(output: &Path, want: &Expected, cancel: &AtomicBool) -> Result<()> {
    let mut file = File::open(output).map_err(|e| io_error(output, e))?;
    let layout = read_layout(&mut file, output)?;
    drop(file);
    let types: Vec<u8> = layout.blocks.iter().map(|b| b.block_type).collect();
    if layout.frames_start != want.frames_start || types != want.block_types {
        return Err(verify_failed(
            output,
            "the metadata blocks are not as planned",
        ));
    }
    if layout.streaminfo != want.info {
        return Err(verify_failed(output, "STREAMINFO is not as written"));
    }
    let frames = want.frames_start..want.frames_start + want.frames_bytes;
    let info = &layout.streaminfo;
    let decoded = decode_frames(
        output,
        &layout.streaminfo_bytes,
        frames,
        u16::from(info.channels),
        u16::from(info.bits),
        cancel,
    )?;
    if decoded.frames != info.total_samples {
        return Err(verify_failed(
            output,
            &format!(
                "{} samples per channel decoded, {} written",
                decoded.frames, info.total_samples
            ),
        ));
    }
    if decoded.pcm_hash != want.pcm_hash {
        return Err(verify_failed(
            output,
            "the decoded samples differ from the encoded ones",
        ));
    }
    if decoded.md5 != info.md5 {
        return Err(verify_failed(
            output,
            "the decoded samples do not match the STREAMINFO MD5",
        ));
    }
    tracing::debug!(path = %output.display(), stage = "verify", frames = decoded.frames, "verified");
    Ok(())
}
