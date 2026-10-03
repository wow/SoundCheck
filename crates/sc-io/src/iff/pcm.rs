//! Streaming PCM reader over [`AudioFormat::data`]: fixed blocks of [`BLOCK_FRAMES`] frames,
//! one buffer of at most `min(BLOCK_FRAMES * block_align, audio bytes)` bytes, sequential reads
//! only.
//!
//! Units: integer samples are returned as `i32` holding the stored value at its valid bit depth
//! (a 24-bit sample is in -8388608..=8388607; 24 valid bits in a 32-bit container are shifted
//! down by 8; WAV 8-bit offset binary becomes -128..=127), never rescaled. Float samples are
//! returned as `f64` holding the stored value exactly (an `f32` widens losslessly), nominal
//! full scale -1.0..1.0 but not clamped. Blocks are interleaved, `channels` values per frame.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sc_core::{Error, Result};

use super::format::{AudioFormat, SampleEncoding};
use super::io_error;

/// Frames per block returned by [`PcmReader`] (the last block may be shorter).
pub const BLOCK_FRAMES: usize = 4096;

/// Reads the samples of one file block by block.
pub struct PcmReader<R> {
    reader: R,
    path: PathBuf,
    encoding: SampleEncoding,
    /// Container bytes per sample.
    width: usize,
    /// Right shift that brings a left-aligned `i32` to the valid bit depth.
    shift: u32,
    channels: usize,
    block_align: usize,
    block_frames: usize,
    frames_left: u64,
    buf: Vec<u8>,
}

impl<R: Read + Seek> PcmReader<R> {
    /// Positions `reader` at the start of `format.data`.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when `format` breaks the invariants documented on
    /// [`AudioFormat`] (only possible for a hand-built value); [`Error::Io`] when seeking fails.
    pub fn new(mut reader: R, format: &AudioFormat, path: &Path) -> Result<Self> {
        let width = usize::from(format.bits_per_sample / 8);
        let channels = usize::from(format.channels);
        let block_align = usize::from(format.block_align);
        let width_ok = match format.encoding {
            SampleEncoding::UnsignedInt8 => width == 1,
            SampleEncoding::IntLe | SampleEncoding::IntBe => (1..=4).contains(&width),
            SampleEncoding::FloatLe32 | SampleEncoding::FloatBe32 => width == 4,
            SampleEncoding::FloatLe64 | SampleEncoding::FloatBe64 => width == 8,
        };
        let valid_ok = (1..=format.bits_per_sample).contains(&format.valid_bits)
            && (!format.encoding.is_float() || format.valid_bits == format.bits_per_sample);
        if !width_ok
            || !valid_ok
            || !format.bits_per_sample.is_multiple_of(8)
            || channels == 0
            || block_align != channels * width
        {
            return Err(Error::InvalidArgument(format!(
                "inconsistent audio format {format:?}"
            )));
        }
        let data_len = format.data.end.saturating_sub(format.data.start);
        let frames = format.frames.min(data_len / block_align as u64);
        let block_frames = usize::try_from(frames.min(BLOCK_FRAMES as u64)).unwrap_or(BLOCK_FRAMES);
        reader
            .seek(SeekFrom::Start(format.data.start))
            .map_err(|source| io_error(path, source))?;
        Ok(Self {
            reader,
            path: path.to_path_buf(),
            encoding: format.encoding,
            width,
            shift: 32 - u32::from(format.valid_bits).min(32),
            channels,
            block_align,
            block_frames,
            frames_left: frames,
            buf: vec![0; block_frames * block_align],
        })
    }

    /// Whether the source holds floats (read them with [`Self::next_block_float`]).
    #[must_use]
    pub fn is_float(&self) -> bool {
        self.encoding.is_float()
    }

    /// Frames not yet returned.
    #[must_use]
    pub fn frames_remaining(&self) -> u64 {
        self.frames_left
    }

    /// Size of the internal block buffer, bytes: never more than the audio bytes in the file.
    #[must_use]
    pub fn buffer_len_bytes(&self) -> usize {
        self.buf.len()
    }

    /// Reads the next block of integer samples into `out` (cleared, then filled with
    /// `frames * channels` interleaved values). Returns the frames read, 0 at the end.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for a float source; [`Error::Corrupt`] when the file ends
    /// inside the audio (it changed after the walk); [`Error::Io`] when reading fails.
    pub fn next_block_int(&mut self, out: &mut Vec<i32>) -> Result<usize> {
        if self.is_float() {
            return Err(Error::InvalidArgument(
                "integer read from a float source".into(),
            ));
        }
        let frames = self.fill()?;
        let bytes = &self.buf[..frames * self.block_align];
        out.clear();
        out.resize(frames * self.channels, 0);
        let s = self.shift;
        match (self.encoding, self.width) {
            (SampleEncoding::UnsignedInt8, _) => fill_int::<1, false>(bytes, out, s, 0x80),
            (SampleEncoding::IntLe, 1) => fill_int::<1, false>(bytes, out, s, 0),
            (SampleEncoding::IntLe, 2) => fill_int::<2, false>(bytes, out, s, 0),
            (SampleEncoding::IntLe, 3) => fill_int::<3, false>(bytes, out, s, 0),
            (SampleEncoding::IntLe, _) => fill_int::<4, false>(bytes, out, s, 0),
            (SampleEncoding::IntBe, 1) => fill_int::<1, true>(bytes, out, s, 0),
            (SampleEncoding::IntBe, 2) => fill_int::<2, true>(bytes, out, s, 0),
            (SampleEncoding::IntBe, 3) => fill_int::<3, true>(bytes, out, s, 0),
            _ => fill_int::<4, true>(bytes, out, s, 0),
        }
        Ok(frames)
    }

    /// Reads the next block of float samples into `out` (cleared, then filled with
    /// `frames * channels` interleaved values). Returns the frames read, 0 at the end.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for an integer source; otherwise as for
    /// [`Self::next_block_int`].
    pub fn next_block_float(&mut self, out: &mut Vec<f64>) -> Result<usize> {
        if !self.is_float() {
            return Err(Error::InvalidArgument(
                "float read from an integer source".into(),
            ));
        }
        let frames = self.fill()?;
        let bytes = &self.buf[..frames * self.block_align];
        out.clear();
        out.resize(frames * self.channels, 0.0);
        match self.encoding {
            SampleEncoding::FloatLe32 => fill_f32::<false>(bytes, out),
            SampleEncoding::FloatBe32 => fill_f32::<true>(bytes, out),
            SampleEncoding::FloatLe64 => fill_f64::<false>(bytes, out),
            _ => fill_f64::<true>(bytes, out),
        }
        Ok(frames)
    }

    /// Reads the next block's bytes into the buffer; returns its frames.
    fn fill(&mut self) -> Result<usize> {
        let frames = usize::try_from(self.frames_left.min(self.block_frames as u64))
            .unwrap_or(self.block_frames);
        if frames == 0 {
            return Ok(0);
        }
        let bytes = &mut self.buf[..frames * self.block_align];
        self.reader.read_exact(bytes).map_err(|source| {
            if source.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::Corrupt {
                    path: self.path.clone(),
                    detail: "the file ends inside its audio data".into(),
                }
            } else {
                io_error(&self.path, source)
            }
        })?;
        self.frames_left -= frames as u64;
        Ok(frames)
    }
}

/// Decodes `W`-byte integers: the bytes go to the top of an `i32` (most significant first),
/// `flip` turns offset binary into two's complement, an arithmetic shift drops the container
/// padding and sign-extends.
fn fill_int<const W: usize, const BIG: bool>(bytes: &[u8], out: &mut [i32], shift: u32, flip: u8) {
    for (c, o) in bytes.as_chunks::<W>().0.iter().zip(out.iter_mut()) {
        let mut a = [0_u8; 4];
        let v = if BIG {
            a[..W].copy_from_slice(c);
            a[0] ^= flip;
            i32::from_be_bytes(a)
        } else {
            a[4 - W..].copy_from_slice(c);
            a[3] ^= flip;
            i32::from_le_bytes(a)
        };
        *o = v >> shift;
    }
}

fn fill_f32<const BIG: bool>(bytes: &[u8], out: &mut [f64]) {
    for (c, o) in bytes.as_chunks::<4>().0.iter().zip(out.iter_mut()) {
        let a = *c;
        *o = f64::from(if BIG {
            f32::from_be_bytes(a)
        } else {
            f32::from_le_bytes(a)
        });
    }
}

fn fill_f64<const BIG: bool>(bytes: &[u8], out: &mut [f64]) {
    for (c, o) in bytes.as_chunks::<8>().0.iter().zip(out.iter_mut()) {
        let a = *c;
        *o = if BIG {
            f64::from_be_bytes(a)
        } else {
            f64::from_le_bytes(a)
        };
    }
}

/// Every integer sample of a file, interleaved (wraps [`PcmReader::next_block_int`]).
///
/// # Errors
/// As for [`PcmReader::new`] and [`PcmReader::next_block_int`].
pub fn read_all_int<R: Read + Seek>(
    reader: R,
    format: &AudioFormat,
    path: &Path,
) -> Result<Vec<i32>> {
    let mut pcm = PcmReader::new(reader, format, path)?;
    let mut all = Vec::new();
    let mut block = Vec::new();
    while pcm.next_block_int(&mut block)? > 0 {
        all.extend_from_slice(&block);
    }
    Ok(all)
}

/// Every float sample of a file, interleaved (wraps [`PcmReader::next_block_float`]).
///
/// # Errors
/// As for [`PcmReader::new`] and [`PcmReader::next_block_float`].
pub fn read_all_float<R: Read + Seek>(
    reader: R,
    format: &AudioFormat,
    path: &Path,
) -> Result<Vec<f64>> {
    let mut pcm = PcmReader::new(reader, format, path)?;
    let mut all = Vec::new();
    let mut block = Vec::new();
    while pcm.next_block_float(&mut block)? > 0 {
        all.extend_from_slice(&block);
    }
    Ok(all)
}

#[cfg(test)]
mod tests;
