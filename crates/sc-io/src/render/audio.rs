//! The audio stage: the samples after the trim (snapped back and faded in as [`super::head`]
//! describes), through gain and requantisation ([`sc_dsp::Requantiser`]), encoded and written
//! while a BLAKE3 hash of the written bytes is kept (the tee hash a verifier compares against). Memory is one block of
//! [`crate::iff::BLOCK_FRAMES`] frames whatever the file length. A cancel flag is checked once
//! per block.
//!
//! The dither seed is derived from the source and the request, never from the clock: BLAKE3
//! over a domain label, the source's format chunk payload, its frame count, the first
//! [`SEED_FRAMES`] frames of samples (before the trim; fewer when the file is shorter) and the
//! request (gain, trim, output depth). It does not depend on how the samples are read in
//! blocks, so the same file and request always give the same bytes.

use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sc_core::{Error, Result};
use sc_dsp::{FadeIn, Requantiser, SourceDepth};

use super::head::{self, HeadSnap};
use crate::iff::{AudioFormat, PcmReader, encode_samples};

/// Frames hashed into the dither seed (about 1.4 s at 48 kHz).
pub const SEED_FRAMES: u64 = 65_536;

/// What the audio stage did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AudioDone {
    /// BLAKE3 of the audio bytes as written.
    pub pcm_hash: [u8; 32],
    /// Whether TPDF dither was added.
    pub dithered: bool,
    /// Whether the samples were copied exactly (0 dB, no loss of depth).
    pub exact: bool,
    /// Samples set to the largest or smallest code by less than one step.
    pub saturated: u64,
}

/// How to render the audio.
#[derive(Debug, Clone, Copy)]
pub(super) struct AudioJob<'a> {
    /// The source (named in read errors).
    pub input: &'a Path,
    /// The output (named in write errors).
    pub output: &'a Path,
    pub format: &'a AudioFormat,
    pub trim_frames: u64,
    pub frames_out: u64,
    pub bits: u16,
    pub big_endian: bool,
    pub gain_db: f64,
    /// TPDF seed, from [`dither_seed`].
    pub seed: u64,
    /// Set by another thread to stop the render.
    pub cancel: &'a AtomicBool,
}

/// Samples of one block, integer or float.
enum Block {
    Int(Vec<i32>),
    Float(Vec<f64>),
}

impl Block {
    fn new(format: &AudioFormat) -> Self {
        if format.encoding.is_float() {
            Self::Float(Vec::new())
        } else {
            Self::Int(Vec::new())
        }
    }

    fn read<R: Read + Seek>(&mut self, reader: &mut PcmReader<R>) -> Result<usize> {
        match self {
            Self::Int(v) => reader.next_block_int(v),
            Self::Float(v) => reader.next_block_float(v),
        }
    }
}

/// [`Error::Cancelled`] once `cancel` is set.
pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Refuses a source whose padding bits hold data.
fn check_padding<R: Read + Seek>(reader: &PcmReader<R>, path: &Path) -> Result<()> {
    match reader.padding_bits_nonzero() {
        0 => Ok(()),
        n => Err(Error::UnsupportedFormat {
            path: path.to_path_buf(),
            detail: format!(
                "{n} samples hold data in the container's padding bits, which the format \
                 requires to be zero; the samples are not what the header says"
            ),
        }),
    }
}

/// The most positive and most negative sample, as fractions of full scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Peaks {
    /// Largest value, >= 0.
    pub max: f64,
    /// Smallest value, <= 0.
    pub min: f64,
}

/// The head cut actually made for a requested cut of `requested` frames (see
/// [`super::head`]): `requested` itself when it is 0 or leaves no audio (refused later).
///
/// # Errors
/// [`Error::Cancelled`]; reading errors.
pub(super) fn snapped_trim<R: Read + Seek>(
    src: R,
    path: &Path,
    format: &AudioFormat,
    requested: u64,
    cancel: &AtomicBool,
) -> Result<u64> {
    if requested == 0 || requested >= format.frames {
        return Ok(requested);
    }
    let mut snap = HeadSnap::new(requested, format.sample_rate, format.channels);
    let mut reader = PcmReader::new(src, format, path)?;
    let mut block = Block::new(format);
    while snap.wants_more() {
        check_cancel(cancel)?;
        if block.read(&mut reader)? == 0 {
            break;
        }
        match &block {
            Block::Int(v) => snap.push(v, head::int_magnitude),
            Block::Float(v) => snap.push(v, head::float_magnitude),
        }
    }
    Ok(snap.finish())
}

/// The fade-in of a render that cuts `trim_frames` (none without a cut).
///
/// # Errors
/// [`Error::InvalidArgument`] for 0 channels.
pub(super) fn head_fade(
    trim_frames: u64,
    sample_rate_hz: u32,
    channels: u16,
) -> Result<Option<FadeIn>> {
    if trim_frames == 0 {
        return Ok(None);
    }
    FadeIn::new(head::head_fade_frames(sample_rate_hz), channels).map(Some)
}

/// The signed peaks of the samples after the trim.
///
/// # Errors
/// [`Error::Corrupt`] for a float sample that is NaN or infinite; [`Error::UnsupportedFormat`]
/// for data in padding bits; [`Error::Cancelled`]; reading errors.
pub(super) fn peaks_after_trim<R: Read + Seek>(
    src: R,
    path: &Path,
    format: &AudioFormat,
    trim_frames: u64,
    cancel: &AtomicBool,
) -> Result<Peaks> {
    let mut reader = PcmReader::new(src, format, path)?;
    let mut block = Block::new(format);
    let channels = u64::from(format.channels);
    let mut skip = trim_frames.saturating_mul(channels);
    let (mut max_int, mut min_int) = (0_i32, 0_i32);
    let mut peaks = Peaks { max: 0.0, min: 0.0 };
    loop {
        check_cancel(cancel)?;
        let frames = block.read(&mut reader)?;
        if frames == 0 {
            break;
        }
        check_padding(&reader, path)?;
        let n = frames as u64 * channels;
        let start = usize::try_from(skip.min(n)).unwrap_or(0);
        skip -= start as u64;
        match &block {
            Block::Int(v) => {
                for x in &v[start..] {
                    max_int = max_int.max(*x);
                    min_int = min_int.min(*x);
                }
            }
            Block::Float(v) => {
                for x in &v[start..] {
                    if !x.is_finite() {
                        return Err(Error::Corrupt {
                            path: path.to_path_buf(),
                            detail: "a float sample is NaN or infinite".into(),
                        });
                    }
                    peaks.max = peaks.max.max(*x);
                    peaks.min = peaks.min.min(*x);
                }
            }
        }
    }
    if let Block::Int(_) = block {
        // The full-scale divisor is a power of two, so the quotients are exact.
        let full = 2_f64.powi(i32::from(format.valid_bits) - 1);
        peaks = Peaks {
            max: f64::from(max_int) / full,
            min: f64::from(min_int) / full,
        };
    }
    Ok(peaks)
}

/// The request's part of the dither seed.
#[derive(Debug, Clone, Copy)]
pub(super) struct SeedRequest {
    pub gain_db: f64,
    pub trim_frames: u64,
    pub bits: u16,
}

/// The dither seed (see the module documentation).
///
/// # Errors
/// [`Error::Cancelled`]; reading errors.
pub(super) fn dither_seed<R: Read + Seek>(
    src: R,
    path: &Path,
    format: &AudioFormat,
    format_payload: &[u8],
    req: SeedRequest,
    cancel: &AtomicBool,
) -> Result<u64> {
    let mut h = seed_hasher(format_payload, format.frames, req);
    let mut reader = PcmReader::new(src, format, path)?;
    let mut block = Block::new(format);
    let channels = usize::from(format.channels);
    let mut left = usize::try_from(SEED_FRAMES.min(format.frames)).unwrap_or(0) * channels;
    while left > 0 {
        check_cancel(cancel)?;
        if block.read(&mut reader)? == 0 {
            break;
        }
        match &block {
            Block::Int(v) => {
                let take = v.len().min(left);
                for x in &v[..take] {
                    h.update(&x.to_le_bytes());
                }
                left -= take;
            }
            Block::Float(v) => {
                let take = v.len().min(left);
                for x in &v[..take] {
                    h.update(&x.to_bits().to_le_bytes());
                }
                left -= take;
            }
        }
    }
    Ok(seed_of(&h))
}

/// The seed hash before any sample: the domain label, the format payload, the frame count
/// and the request. The samples follow as little-endian `i32` (or `f64` bits for a float
/// source), then [`seed_of`] gives the seed.
pub(super) fn seed_hasher(format_payload: &[u8], frames: u64, req: SeedRequest) -> blake3::Hasher {
    let mut h = blake3::Hasher::new();
    h.update(b"SoundCheck TPDF seed 2");
    h.update(&(format_payload.len() as u64).to_le_bytes());
    h.update(format_payload);
    h.update(&frames.to_le_bytes());
    h.update(&req.gain_db.to_bits().to_le_bytes());
    h.update(&req.trim_frames.to_le_bytes());
    h.update(&req.bits.to_le_bytes());
    h
}

/// The seed: the first eight bytes of the hash, little-endian.
pub(super) fn seed_of(h: &blake3::Hasher) -> u64 {
    let mut seed = [0_u8; 8];
    seed.copy_from_slice(&h.finalize().as_bytes()[..8]);
    u64::from_le_bytes(seed)
}

/// Streams the audio of `job` from `src` into `w`.
///
/// # Errors
/// [`Error::UnsupportedFormat`] for data in padding bits; [`Error::Corrupt`] when the source
/// yields fewer frames than its header promised; [`Error::Cancelled`]; [`Error::Io`] naming the
/// input for read errors and the output for write errors.
pub(super) fn write_audio<R: Read + Seek, W: Write>(
    src: R,
    w: &mut W,
    job: &AudioJob<'_>,
) -> Result<AudioDone> {
    let path = job.input;
    let format = job.format;
    let mut reader = PcmReader::new(src, format, path)?;
    let mut block = Block::new(format);
    let channels = usize::from(format.channels);
    let mut skip = job.trim_frames.saturating_mul(channels as u64);
    let source = if format.encoding.is_float() {
        SourceDepth::Float
    } else {
        SourceDepth::Int {
            bits: format.valid_bits,
        }
    };
    let mut quant = Requantiser::new(source, job.bits, job.gain_db, job.seed)?;
    if let Some(fade) = head_fade(job.trim_frames, format.sample_rate, format.channels)? {
        quant.set_fade_in(fade);
    }
    let mut out = vec![0_i32; crate::iff::BLOCK_FRAMES * channels];
    let mut bytes = Vec::with_capacity(out.len() * usize::from(job.bits / 8));
    let mut hasher = blake3::Hasher::new();
    let mut written = 0_u64;
    loop {
        check_cancel(job.cancel)?;
        let frames = block.read(&mut reader)?;
        if frames == 0 {
            break;
        }
        check_padding(&reader, path)?;
        let read = frames * channels;
        let start = usize::try_from(skip.min(read as u64)).unwrap_or(0);
        skip -= start as u64;
        let kept = read - start;
        match &block {
            Block::Int(v) => quant.push_int(&v[start..read], &mut out[..kept])?,
            Block::Float(v) => quant.push_float(&v[start..read], &mut out[..kept])?,
        }
        encode_samples(&out[..kept], job.bits, job.big_endian, &mut bytes);
        hasher.update(&bytes);
        w.write_all(&bytes).map_err(|source| Error::Io {
            path: job.output.to_path_buf(),
            source,
        })?;
        written += (kept / channels) as u64;
    }
    if written != job.frames_out {
        return Err(Error::Corrupt {
            path: path.to_path_buf(),
            detail: format!(
                "{written} frames of audio read, {} expected (the file changed?)",
                job.frames_out
            ),
        });
    }
    Ok(AudioDone {
        pcm_hash: *hasher.finalize().as_bytes(),
        dithered: quant.is_dithered(),
        exact: quant.is_exact(),
        saturated: quant.samples_saturated(),
    })
}

#[cfg(test)]
mod tests;
