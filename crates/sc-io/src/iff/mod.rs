//! Reading WAV, RF64, AIFF and AIFF-C files exactly: the chunk table, the audio format and the
//! stored PCM, as the ground the byte-preserving writers stand on.
//!
//! Specifications: Microsoft/IBM "Multimedia Programming Interface and Data Specifications 1.0"
//! (1991, RIFF and `WAVE`), Microsoft's `WAVEFORMATEXTENSIBLE` documentation (valid bits, channel
//! mask, sub-format GUID), EBU Tech 3306 (RF64, `ds64`), Apple "Audio Interchange File Format"
//! 1.3 (1989, `FORM`/`AIFF`, `COMM` with its 80-bit IEEE 754 extended sample rate, `SSND` with
//! offset and block size) and the Apple AIFF-C draft (1991, compression types).
//!
//! - [`walk`] lists every chunk with its header offset, raw size field, payload byte range and
//!   pad byte, never trusting a size: nothing is allocated from a declared size and nothing is
//!   read past the end of the file. It tolerates what real files do: an odd-length chunk
//!   (often `data`) without its pad byte, stray bytes after the container (an `ID3v1` tag), a
//!   container size that disagrees with the file length (chunks past it are still walked), and
//!   a file cut inside a chunk (clamped, flagged).
//! - [`read_format`] decodes `fmt ` (PCM, IEEE float, extensible) or `COMM` + `SSND` into an
//!   [`AudioFormat`] whose `data` range holds exactly the audio bytes.
//! - [`PcmReader`] streams the samples in fixed blocks of [`BLOCK_FRAMES`] frames: floats as
//!   `f64` (bit-exact), integers as `i32` at their valid bit depth. Bits below the valid depth
//!   (container padding, which the specifications require to be zero) are dropped, and every
//!   sample where they were not zero is counted, so a non-standard layout (a right-justified
//!   24-in-32 file, data in padding bits) is detected rather than silently read.
//! - The `write` helpers produce the only bytes an output does not copy from its source:
//!   container and chunk headers, a plain-PCM `fmt `, a plain AIFF `COMM` and integer samples
//!   (used by [`crate::render`]).

mod aiff;
mod ds64;
mod extended;
mod format;
mod pad;
mod pcm;
mod walk;
mod wave;
mod write;

#[cfg(test)]
pub(crate) mod test_build;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sc_core::{Error, Result};

pub use extended::{ExtendedRateError, extended_from_sample_rate, sample_rate_from_extended};
pub use format::{
    AudioFormat, MAX_SAMPLE_RATE_HZ, MIN_SAMPLE_RATE_HZ, SampleEncoding, read_format,
};
pub use pcm::{BLOCK_FRAMES, PcmReader};
pub use walk::{
    Chunk, ChunkTable, Ds64, Ds64Entry, MAX_CHUNKS, MAX_DS64_ENTRIES, walk, walk_bytes,
};
pub use write::{
    AIFF_COMM_BYTES, OutContainer, SSND_FIELDS_BYTES, WAVE_FMT_PCM_BYTES, aiff_comm, chunk_header,
    container_header, encode_samples, wave_fmt_pcm,
};

/// Path used in errors for data read from memory.
pub const MEMORY_PATH: &str = "<memory>";

/// The container family, from the first twelve bytes of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Container {
    /// `RIFF` ... `WAVE`: little-endian chunks, 32-bit sizes.
    Riff,
    /// `RF64` (or the identical `BW64` of ITU-R BS.2088) ... `WAVE`: 64-bit sizes in `ds64`.
    Rf64,
    /// `FORM` ... `AIFF`: big-endian chunks.
    Aiff,
    /// `FORM` ... `AIFC`: big-endian chunks, compression type in `COMM`.
    Aifc,
}

impl Container {
    /// Whether chunk sizes (and, for AIFF, samples) are big-endian.
    #[must_use]
    pub fn is_big_endian(self) -> bool {
        matches!(self, Self::Aiff | Self::Aifc)
    }

    /// Whether this is a `WAVE` form (RIFF or RF64).
    #[must_use]
    pub fn is_wave(self) -> bool {
        matches!(self, Self::Riff | Self::Rf64)
    }
}

/// A file's chunk table and audio format, read together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffHeader {
    /// Every chunk, in file order.
    pub table: ChunkTable,
    /// The audio format and where the audio bytes are.
    pub format: AudioFormat,
}

/// Walks `reader` and decodes its format: [`walk`] then [`read_format`].
///
/// # Errors
/// As for [`walk`] and [`read_format`].
pub fn read_header<R: Read + Seek>(reader: &mut R, path: &Path) -> Result<IffHeader> {
    let table = walk(reader, path)?;
    let format = read_format(reader, &table, path)?;
    Ok(IffHeader { table, format })
}

/// Bounded random access to a reader whose length is known; every read is checked against it.
pub(crate) struct Source<'a, R> {
    reader: &'a mut R,
    path: &'a Path,
    /// File length, bytes.
    pub len: u64,
}

impl<'a, R: Read + Seek> Source<'a, R> {
    /// Measures the length of `reader`.
    pub fn new(reader: &'a mut R, path: &'a Path) -> Result<Self> {
        let len = reader
            .seek(SeekFrom::End(0))
            .map_err(|source| io_error(path, source))?;
        Ok(Self { reader, path, len })
    }

    /// Fills `buf` from byte `offset`.
    ///
    /// # Errors
    /// [`Error::Corrupt`] when the range runs past the end of the file (callers check first, so
    /// this means the file changed underneath), [`Error::Io`] when reading fails.
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let end = offset.checked_add(buf.len() as u64);
        if end.is_none_or(|e| e > self.len) {
            return Err(corrupt(self.path, offset, "read past the end of the file"));
        }
        self.reader
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.reader.read_exact(buf))
            .map_err(|source| io_error(self.path, source))
    }

    /// The path used in errors.
    pub fn path(&self) -> &'a Path {
        self.path
    }
}

/// An [`Error::Corrupt`] naming the byte offset of the problem.
pub(crate) fn corrupt(path: &Path, offset: u64, reason: &str) -> Error {
    Error::Corrupt {
        path: path.to_path_buf(),
        detail: format!("{reason} (at byte {offset})"),
    }
}

/// An [`Error::UnsupportedFormat`] with `detail`.
pub(crate) fn unsupported(path: &Path, detail: String) -> Error {
    Error::UnsupportedFormat {
        path: path.to_path_buf(),
        detail,
    }
}

/// An [`Error::Io`] for `path`.
pub(crate) fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: PathBuf::from(path),
        source,
    }
}

/// A four-character code for messages: printable ASCII as is, other bytes escaped.
pub(crate) fn fourcc(id: [u8; 4]) -> String {
    id.iter()
        .flat_map(|b| b.escape_ascii())
        .map(char::from)
        .collect()
}

/// `value` as `usize`, at most `cap`.
pub(crate) fn capped(value: u64, cap: usize) -> usize {
    usize::try_from(value).map_or(cap, |v| v.min(cap))
}
