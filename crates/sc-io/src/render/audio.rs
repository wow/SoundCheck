//! The audio stage: the samples after the trim, through gain and requantisation
//! ([`sc_dsp::Requantiser`]), encoded and written while a BLAKE3 hash of the written bytes is
//! kept (the tee hash a verifier compares against). Memory is one block of
//! [`crate::iff::BLOCK_FRAMES`] frames whatever the file length.
//!
//! The dither seed is derived from the source alone, never from the clock: BLAKE3 over a
//! domain label, the source's format chunk payload and the first block of samples (before the
//! trim), so the same file and request always give the same bytes.

use std::io::{Read, Seek, Write};
use std::path::Path;

use sc_core::{Error, Result};
use sc_dsp::{Requantiser, SourceDepth};

use crate::iff::{AudioFormat, PcmReader, encode_samples};

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
    pub path: &'a Path,
    pub format: &'a AudioFormat,
    pub trim_frames: u64,
    pub frames_out: u64,
    pub bits: u16,
    pub big_endian: bool,
    pub gain_db: f64,
    /// The source's format chunk payload (part of the dither seed).
    pub format_payload: &'a [u8],
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

/// The largest absolute sample after the trim, as a fraction of full scale.
///
/// # Errors
/// [`Error::Corrupt`] for a float sample that is NaN or infinite; [`Error::UnsupportedFormat`]
/// for data in padding bits; reading errors.
pub(super) fn peak_after_trim<R: Read + Seek>(
    src: R,
    path: &Path,
    format: &AudioFormat,
    trim_frames: u64,
) -> Result<f64> {
    let mut reader = PcmReader::new(src, format, path)?;
    let mut block = Block::new(format);
    let channels = u64::from(format.channels);
    let mut skip = trim_frames.saturating_mul(channels);
    let (mut peak_int, mut peak_float) = (0_i64, 0.0_f64);
    loop {
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
                    peak_int = peak_int.max(i64::from(*x).abs());
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
                    peak_float = peak_float.max(x.abs());
                }
            }
        }
    }
    Ok(match block {
        // Integer peaks are below 2^31, exact in f64; the scale is a power of two.
        #[allow(clippy::cast_precision_loss)]
        Block::Int(_) => peak_int as f64 / 2_f64.powi(i32::from(format.valid_bits) - 1),
        Block::Float(_) => peak_float,
    })
}

/// The dither seed: BLAKE3 of a label, the format payload and the first block's samples.
fn dither_seed(format_payload: &[u8], block: &Block) -> u64 {
    let mut h = blake3::Hasher::new();
    h.update(b"SoundCheck TPDF seed 1");
    h.update(format_payload);
    match block {
        Block::Int(v) => {
            for x in v {
                h.update(&x.to_le_bytes());
            }
        }
        Block::Float(v) => {
            for x in v {
                h.update(&x.to_le_bytes());
            }
        }
    }
    let mut seed = [0_u8; 8];
    seed.copy_from_slice(&h.finalize().as_bytes()[..8]);
    u64::from_le_bytes(seed)
}

/// Streams the audio of `job` from `src` into `w`.
///
/// # Errors
/// [`Error::UnsupportedFormat`] for data in padding bits; [`Error::Corrupt`] when the source
/// yields fewer frames than its header promised; reading and writing errors.
pub(super) fn write_audio<R: Read + Seek, W: Write>(
    src: R,
    w: &mut W,
    job: &AudioJob<'_>,
) -> Result<AudioDone> {
    let path = job.path;
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
    let mut quantiser: Option<Requantiser> = None;
    let mut out = vec![0_i32; crate::iff::BLOCK_FRAMES * channels];
    let mut bytes = Vec::with_capacity(out.len() * usize::from(job.bits / 8));
    let mut hasher = blake3::Hasher::new();
    let mut written = 0_u64;
    loop {
        let frames = block.read(&mut reader)?;
        if frames == 0 {
            break;
        }
        check_padding(&reader, path)?;
        let quant = match &mut quantiser {
            Some(q) => q,
            None => quantiser.insert(Requantiser::new(
                source,
                job.bits,
                job.gain_db,
                dither_seed(job.format_payload, &block),
            )?),
        };
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
            path: path.to_path_buf(),
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
        dithered: quantiser.as_ref().is_some_and(Requantiser::is_dithered),
        exact: quantiser.as_ref().is_some_and(Requantiser::is_exact),
        saturated: quantiser.as_ref().map_or(0, Requantiser::samples_saturated),
    })
}
