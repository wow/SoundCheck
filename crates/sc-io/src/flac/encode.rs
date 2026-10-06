//! FLAC frames from integer samples, through the `flac-codec` encoder in frames-only mode
//! (its `FlacStreamWriter`: no metadata, one frame per call, fixed block size, frame numbers
//! counted from 0), with the bookkeeping STREAMINFO and SEEKTABLE need.
//!
//! The encoder runs with its default settings (block size 4096, LPC order up to 8, residual
//! partition order up to 5, the cheapest of independent, left/side, side/right and mid/side
//! coding per frame) and without its optional thread pool, so it is single-threaded and the
//! same samples always give the same bytes.
//!
//! Two guards stand between the encoder and the file:
//! - every sample must lie inside the output range (`-2^(bits-1)..2^(bits-1)`), checked
//!   before encoding (a sample outside it would be a bug upstream, and some encoders write a
//!   wrong frame for one rather than fail);
//! - no frame may be larger than [`max_frame_bytes`], the size of the same frame stored
//!   uncompressed (VERBATIM), which a correct encoder never exceeds because it falls back to
//!   VERBATIM whenever prediction does not pay.
//!
//! The MD5 of the samples (RFC 9639 section 8.2: interleaved, little-endian, `bits / 8` bytes
//! each) and a BLAKE3 hash of the same bytes are computed while encoding: the first goes into
//! STREAMINFO, the second is what verification compares a fresh decode against.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use flac_codec::encode::{FlacStreamWriter, Options};
use md5::{Digest, Md5};
use sc_core::{Error, Result};

use super::le_sample_bytes;

/// Samples per channel in every frame but the last.
pub const OUTPUT_BLOCK_FRAMES: usize = 4096;

/// Longest frame header (RFC 9639 section 9.1): 4 fixed bytes, a 7-byte coded number, 2 bytes
/// of block size, 2 of sample rate and the CRC-8.
const FRAME_HEADER_MAX_BYTES: usize = 16;

/// The size of a frame of `block` samples per channel stored uncompressed: the longest header,
/// per channel a subframe header byte and `bits / 8` bytes per sample, one byte of alignment
/// and the CRC-16. No correct frame is larger.
#[must_use]
pub fn max_frame_bytes(channels: u16, block: usize, bits: u16) -> usize {
    let per_channel = 1 + block * usize::from(bits).div_ceil(8);
    FRAME_HEADER_MAX_BYTES + usize::from(channels) * per_channel + 1 + 2
}

/// A [`Write`] the encoder owns that the caller can still read: each frame is collected here,
/// checked, then written to the file.
#[derive(Debug, Clone, Default)]
struct FrameBuf(Rc<RefCell<Vec<u8>>>);

impl Write for FrameBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What the frames hold, for STREAMINFO, SEEKTABLE and verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// Samples per channel.
    pub total_samples: u64,
    /// Smallest frame, bytes.
    pub min_frame: u32,
    /// Largest frame, bytes.
    pub max_frame: u32,
    /// Offset of each frame from the first frame, bytes.
    pub frame_offsets: Vec<u64>,
    /// Length of all frames, bytes.
    pub frames_bytes: u64,
    /// MD5 of the samples (the STREAMINFO signature).
    pub md5: [u8; 16],
    /// BLAKE3 of the same bytes the MD5 covers.
    pub pcm_hash: [u8; 32],
}

/// Encodes interleaved samples into frames of [`OUTPUT_BLOCK_FRAMES`]; streaming
/// (`push` any number of samples, then `finish`), memory bounded by one frame plus the list of
/// frame offsets.
pub struct FrameEncoder {
    writer: FlacStreamWriter<FrameBuf>,
    frame: FrameBuf,
    sample_rate_hz: u32,
    channels: u16,
    bits: u16,
    pending: Vec<i32>,
    bytes: Vec<u8>,
    md5: Md5,
    tee: blake3::Hasher,
    offsets: Vec<u64>,
    written: u64,
    min_frame: usize,
    max_frame: usize,
    total: u64,
    output: PathBuf,
}

impl std::fmt::Debug for FrameEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameEncoder")
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("channels", &self.channels)
            .field("bits", &self.bits)
            .field("frames", &self.offsets.len())
            .finish_non_exhaustive()
    }
}

impl FrameEncoder {
    /// An encoder for `channels` (1 or 2) at `sample_rate_hz` and `bits` (16 or 24) whose
    /// frames go to the file `output` (named in write errors).
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for other channel counts or depths.
    pub fn new(sample_rate_hz: u32, channels: u16, bits: u16, output: &Path) -> Result<Self> {
        if !(1..=2).contains(&channels) || !(bits == 16 || bits == 24) {
            return Err(Error::InvalidArgument(format!(
                "FLAC output of {channels} channels at {bits} bits; 1 or 2 channels at 16 or 24 \
                 bits are written"
            )));
        }
        let frame = FrameBuf::default();
        Ok(Self {
            writer: FlacStreamWriter::new(frame.clone(), Options::default()),
            frame,
            sample_rate_hz,
            channels,
            bits,
            pending: Vec::with_capacity(OUTPUT_BLOCK_FRAMES * usize::from(channels)),
            bytes: Vec::new(),
            md5: Md5::new(),
            tee: blake3::Hasher::new(),
            offsets: Vec::new(),
            written: 0,
            min_frame: usize::MAX,
            max_frame: 0,
            total: 0,
            output: output.to_path_buf(),
        })
    }

    /// Adds interleaved `samples` (whole frames), writing every completed frame to `w`.
    ///
    /// # Errors
    /// [`Error::Internal`] for a sample outside the output range, a partial frame of samples,
    /// an encoder failure or an oversized frame; [`Error::Io`] naming the output when writing
    /// fails.
    pub fn push<W: Write>(&mut self, samples: &[i32], w: &mut W) -> Result<()> {
        let channels = usize::from(self.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(Error::Internal(format!(
                "{} samples are not whole frames of {channels} channels",
                samples.len()
            )));
        }
        let top = 1_i32 << (self.bits - 1);
        if let Some(bad) = samples.iter().find(|s| !(-top..top).contains(*s)) {
            return Err(Error::Internal(format!(
                "sample {bad} lies outside the {}-bit range; nothing was encoded",
                self.bits
            )));
        }
        le_sample_bytes(samples, usize::from(self.bits / 8), &mut self.bytes);
        self.md5.update(&self.bytes);
        self.tee.update(&self.bytes);
        self.total += (samples.len() / channels) as u64;
        let mut rest = samples;
        while !rest.is_empty() {
            let room = OUTPUT_BLOCK_FRAMES * channels - self.pending.len();
            let take = room.min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == OUTPUT_BLOCK_FRAMES * channels {
                self.encode_pending(w)?;
            }
        }
        Ok(())
    }

    /// Encodes the last, shorter frame and returns what the frames hold.
    ///
    /// # Errors
    /// As for [`Self::push`]; [`Error::Internal`] when no sample was pushed.
    pub fn finish<W: Write>(mut self, w: &mut W) -> Result<Encoded> {
        if !self.pending.is_empty() {
            self.encode_pending(w)?;
        }
        if self.offsets.is_empty() {
            return Err(Error::Internal("no samples to encode".into()));
        }
        let size = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(Encoded {
            total_samples: self.total,
            min_frame: size(self.min_frame),
            max_frame: size(self.max_frame),
            frame_offsets: self.offsets,
            frames_bytes: self.written,
            md5: self.md5.finalize().into(),
            pcm_hash: *self.tee.finalize().as_bytes(),
        })
    }

    fn encode_pending<W: Write>(&mut self, w: &mut W) -> Result<()> {
        let channels = usize::from(self.channels);
        let block = self.pending.len() / channels;
        self.frame.0.borrow_mut().clear();
        // Channels are 1 or 2 and bits 16 or 24 (checked in `new`).
        self.writer
            .write(
                self.sample_rate_hz,
                u8::try_from(self.channels).unwrap_or(2),
                u32::from(self.bits),
                &self.pending,
            )
            .map_err(|e| {
                Error::Internal(format!(
                    "the FLAC encoder failed on frame {}: {e}",
                    self.offsets.len()
                ))
            })?;
        let frame = self.frame.0.borrow();
        let bound = max_frame_bytes(self.channels, block, self.bits);
        if frame.len() > bound {
            return Err(Error::Internal(format!(
                "the FLAC encoder wrote frame {} of {} bytes, more than the {bound} bytes of the \
                 same frame uncompressed",
                self.offsets.len(),
                frame.len()
            )));
        }
        w.write_all(&frame).map_err(|source| Error::Io {
            path: self.output.clone(),
            source,
        })?;
        self.offsets.push(self.written);
        self.written += frame.len() as u64;
        self.min_frame = self.min_frame.min(frame.len());
        self.max_frame = self.max_frame.max(frame.len());
        drop(frame);
        self.pending.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
