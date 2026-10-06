//! Decoding a FLAC source's frames to integer samples, exactly, with the `flac-codec` decoder.
//!
//! The decoder is fed a stream made of `fLaC`, the source's STREAMINFO block (flagged last) and
//! the source's frame bytes, so it never parses the other metadata blocks (a block it would
//! reject, such as a malformed picture, never stops a render; those blocks are carried by
//! byte range). Every frame's header CRC-8 and CRC-16 are checked, frame parameters must agree
//! with STREAMINFO, and only the last frame may hold fewer than 15 samples.
//!
//! When STREAMINFO declares the total sample count, decoding stops after exactly that many
//! samples and [`FlacPcm::frames_end`] is the byte where the last frame ends (whatever
//! follows, such as an `ID3v1` tag, is the caller's to carry); otherwise the frames run to the
//! tail tag found by the walk, or to the end of the file. [`FlacPcm::finish`] checks the
//! count and, when asked to and STREAMINFO holds one, the MD5 signature.

use std::cell::Cell;
use std::io::{BufReader, Chain, Cursor, Read, Seek, SeekFrom, Take};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use flac_codec::decode::FlacSampleReader;
use md5::{Digest, Md5};
use sc_core::{Error, Result};

use super::walk::FlacLayout;
use super::{le_sample_bytes, stream_head};

/// Read buffer for the frames, bytes.
const READ_BUFFER_BYTES: usize = 256 << 10;

/// A reader that counts the bytes taken from it.
struct Counted<R> {
    inner: R,
    count: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.set(self.count.get() + n as u64);
        Ok(n)
    }
}

type Stream<R> = Chain<Cursor<Vec<u8>>, Counted<BufReader<Take<R>>>>;

/// The samples of a FLAC source, frame by frame.
pub struct FlacPcm<R: Read> {
    reader: FlacSampleReader<Stream<R>>,
    consumed: Rc<Cell<u64>>,
    frames_start: u64,
    channels: usize,
    total: Option<u64>,
    delivered: u64,
    path: PathBuf,
    md5: Option<(Md5, usize, [u8; 16])>,
    bytes: Vec<u8>,
}

impl<R: Read> std::fmt::Debug for FlacPcm<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlacPcm")
            .field("path", &self.path)
            .field("channels", &self.channels)
            .field("total", &self.total)
            .field("delivered", &self.delivered)
            .finish_non_exhaustive()
    }
}

/// Where the frames ended and how many samples they held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramesRead {
    /// Samples per channel decoded.
    pub frames: u64,
    /// Byte offset just after the last frame.
    pub frames_end: u64,
}

impl<R: Read + Seek> FlacPcm<R> {
    /// Opens the frames of the source `src` (`path` names it in errors) as `layout` describes;
    /// `check_md5` makes [`Self::finish`] compare the samples with the STREAMINFO signature.
    ///
    /// # Errors
    /// [`Error::Io`] when reading fails; [`Error::Corrupt`] when the decoder rejects the
    /// STREAMINFO block.
    pub fn open(mut src: R, path: &Path, layout: &FlacLayout, check_md5: bool) -> Result<Self> {
        let info = &layout.streaminfo;
        let total = (info.total_samples > 0).then_some(info.total_samples);
        let limit = if total.is_some() {
            layout.file_len
        } else {
            layout.frames_limit()
        };
        src.seek(SeekFrom::Start(layout.frames_start))
            .map_err(|source| io_error(path, source))?;
        let head = stream_head(&layout.streaminfo_bytes);
        let consumed = Rc::new(Cell::new(0));
        let frames = Counted {
            inner: BufReader::with_capacity(
                READ_BUFFER_BYTES,
                src.take(limit.saturating_sub(layout.frames_start)),
            ),
            count: Rc::clone(&consumed),
        };
        let reader = FlacSampleReader::new(Cursor::new(head).chain(frames))
            .map_err(|e| codec_error(path, 0, e))?;
        let md5 = (check_md5 && info.has_md5())
            .then(|| (Md5::new(), info.md5_bytes_per_sample(), info.md5));
        Ok(Self {
            reader,
            consumed,
            frames_start: layout.frames_start,
            channels: usize::from(info.channels),
            total,
            delivered: 0,
            path: path.to_path_buf(),
            md5,
            bytes: Vec::new(),
        })
    }
}

impl<R: Read> FlacPcm<R> {
    /// Replaces `out` with the next frame's interleaved samples; returns the samples per
    /// channel, 0 at the end of the stream.
    ///
    /// # Errors
    /// [`Error::Corrupt`] for a frame the decoder rejects or a file ending inside a frame;
    /// [`Error::Io`] when reading fails.
    pub fn next_block(&mut self, out: &mut Vec<i32>) -> Result<usize> {
        out.clear();
        let delivered = self.delivered;
        let path = &self.path;
        let samples = self
            .reader
            .fill_buf()
            .map_err(|e| codec_error(path, delivered, e))?;
        out.extend_from_slice(samples);
        let n = samples.len();
        self.reader.consume(n);
        if let Some((md5, width, _)) = &mut self.md5 {
            le_sample_bytes(out, *width, &mut self.bytes);
            md5.update(&self.bytes);
        }
        let frames = n / self.channels.max(1);
        self.delivered += frames as u64;
        Ok(frames)
    }

    /// Byte offset just after the last frame decoded so far.
    #[must_use]
    pub fn frames_end(&self) -> u64 {
        self.frames_start + self.consumed.get()
    }

    /// Samples per channel delivered so far.
    #[must_use]
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Ends a complete read: checks the sample count against STREAMINFO and, when asked for in
    /// [`Self::open`], the MD5 signature.
    ///
    /// # Errors
    /// [`Error::Corrupt`] when either differs.
    pub fn finish(self) -> Result<FramesRead> {
        if let Some(total) = self.total
            && total != self.delivered
        {
            return Err(Error::Corrupt {
                path: self.path,
                detail: format!(
                    "the frames hold {} samples per channel, STREAMINFO declares {total}",
                    self.delivered
                ),
            });
        }
        if let Some((md5, _, want)) = self.md5
            && <[u8; 16]>::from(md5.finalize()) != want
        {
            return Err(Error::Corrupt {
                path: self.path,
                detail: "the decoded audio does not match the stream's MD5 signature".into(),
            });
        }
        Ok(FramesRead {
            frames: self.delivered,
            frames_end: self.frames_start + self.consumed.get(),
        })
    }
}

fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// A decoder error: I/O errors name the file, a stream ending inside a frame and every
/// format error make it corrupt.
fn codec_error(path: &Path, delivered: u64, e: flac_codec::Error) -> Error {
    match e {
        flac_codec::Error::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
            Error::Corrupt {
                path: path.to_path_buf(),
                detail: format!("the file ends inside the audio frame after sample {delivered}"),
            }
        }
        flac_codec::Error::Io(io) => io_error(path, io),
        other => Error::Corrupt {
            path: path.to_path_buf(),
            detail: format!("audio frame after sample {delivered}: {other}"),
        },
    }
}

#[cfg(test)]
mod tests;
