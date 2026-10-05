//! The audio format of a WAV/RF64 (`fmt ` + `data`) or AIFF/AIFF-C (`COMM` + `SSND`) file and
//! the exact byte range of its audio. Decoding of the two header layouts lives in `wave.rs`
//! (Microsoft `WAVEFORMATEX`/`WAVEFORMATEXTENSIBLE`) and `aiff.rs` (Apple AIFF 1.3, AIFF-C).

use std::io::{Read, Seek};
use std::ops::Range;
use std::path::Path;

use sc_core::Result;

use super::walk::ChunkTable;
use super::{Source, aiff, corrupt, wave};

/// Lowest sample rate accepted, Hz; anything lower is treated as a corrupt header.
pub const MIN_SAMPLE_RATE_HZ: u32 = 1_000;
/// Highest sample rate accepted, Hz (8 x 192 kHz); anything higher is treated as corrupt.
pub const MAX_SAMPLE_RATE_HZ: u32 = 1_536_000;

/// How one sample is stored in its container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleEncoding {
    /// Unsigned offset binary, one byte (WAV 8-bit: 128 is silence).
    UnsignedInt8,
    /// Two's complement, little-endian (WAV; AIFF-C `sowt`), 1 to 4 bytes.
    IntLe,
    /// Two's complement, big-endian (AIFF; AIFF-C `NONE`/`twos`), 1 to 4 bytes.
    IntBe,
    /// IEEE 754 single precision, little-endian (WAV format 3).
    FloatLe32,
    /// IEEE 754 double precision, little-endian (WAV format 3).
    FloatLe64,
    /// IEEE 754 single precision, big-endian (AIFF-C `fl32`/`FL32`).
    FloatBe32,
    /// IEEE 754 double precision, big-endian (AIFF-C `fl64`/`FL64`).
    FloatBe64,
}

impl SampleEncoding {
    /// Whether samples are floating point.
    #[must_use]
    pub fn is_float(self) -> bool {
        matches!(
            self,
            Self::FloatLe32 | Self::FloatLe64 | Self::FloatBe32 | Self::FloatBe64
        )
    }
}

/// The audio format of a file and where its audio bytes are.
///
/// Invariants (checked by [`read_format`]): `channels >= 1`; `bits_per_sample` is 8, 16, 24 or
/// 32 for integers and 32 or 64 for floats; `1 <= valid_bits <= bits_per_sample`;
/// `block_align == channels * bits_per_sample / 8`;
/// `data.end - data.start == frames * block_align`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormat {
    /// Sample rate, Hz, in `MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ`.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
    /// Container width of one sample, bits (a multiple of 8).
    pub bits_per_sample: u16,
    /// Significant bits per sample; when below `bits_per_sample` the value is stored
    /// left-justified (most significant bits) in its container, as both specifications require.
    pub valid_bits: u16,
    /// Byte order and number type.
    pub encoding: SampleEncoding,
    /// Bytes per frame (all channels).
    pub block_align: u16,
    /// Whole frames of audio.
    pub frames: u64,
    /// AIFF only: `numSampleFrames` from `COMM`. When it disagrees with the frames the `SSND`
    /// chunk holds, `frames` is the smaller and `frames_mismatch` is set.
    pub frames_declared: Option<u64>,
    /// `COMM` and `SSND` disagree on the frame count.
    pub frames_mismatch: bool,
    /// Byte range of exactly `frames` frames of audio, inside the payload of the chunk at
    /// `audio_chunk` (after the `SSND` offset and block-size fields and its offset bytes;
    /// excludes any partial trailing frame). An AIFF with no frames may have no `SSND` chunk
    /// (AIFF 1.3); its range is then empty at the end of the `COMM` payload, so the invariant
    /// below holds without a special case and `audio_chunk` says the chunk is absent.
    pub data: Range<u64>,
    /// Index in [`ChunkTable::chunks`] of the `fmt ` or `COMM` chunk that was decoded.
    pub format_chunk: usize,
    /// Index in [`ChunkTable::chunks`] of the `data` or `SSND` chunk holding `data`; `None`
    /// only for an AIFF without frames and without `SSND`.
    pub audio_chunk: Option<usize>,
    /// `WAVEFORMATEXTENSIBLE` `dwChannelMask`.
    pub channel_mask: Option<u32>,
    /// WAV `wFormatTag` (0x0001 PCM, 0x0003 IEEE float, 0xFFFE extensible).
    pub format_tag: Option<u16>,
    /// AIFF-C compression type (`NONE`, `twos`, `sowt`, `fl32`, `FL32`, `fl64`, `FL64`).
    pub aifc_compression: Option<[u8; 4]>,
}

/// Decodes the audio format of a walked file.
///
/// Supported: WAV/RF64 integer PCM (8-bit unsigned, 16/24/32-bit, any valid bits inside a 1-4
/// byte container), IEEE float 32/64, and `WAVE_FORMAT_EXTENSIBLE` with a PCM or float
/// sub-format; AIFF integer PCM of 1-32 bits; AIFF-C `NONE`, `twos`, `sowt`, `fl32`/`FL32` and
/// `fl64`/`FL64`. The first `fmt `/`COMM` and the first `data`/`SSND` chunk count.
///
/// # Errors
/// [`sc_core::Error::Corrupt`] when the format or audio chunk is missing or malformed, or the
/// channel count, block alignment or sample rate is impossible;
/// [`sc_core::Error::UnsupportedFormat`] for other WAV format tags or extensible sub-formats,
/// other AIFF-C compression types (Ableton's `able`, `ima4`, `ulaw`, ...), sample sizes outside
/// the list above and non-integer AIFF sample rates; [`sc_core::Error::Io`] when reading fails.
pub fn read_format<R: Read + Seek>(
    reader: &mut R,
    table: &ChunkTable,
    path: &Path,
) -> Result<AudioFormat> {
    let mut src = Source::new(reader, path)?;
    if table.container.is_wave() {
        wave::read(&mut src, table)
    } else {
        aiff::read(&mut src, table)
    }
}

/// Checks a decoded rate against the plausible range.
pub(super) fn check_rate(path: &Path, offset: u64, hz: u32) -> Result<u32> {
    if (MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&hz) {
        Ok(hz)
    } else {
        Err(corrupt(
            path,
            offset,
            &format!("sample rate {hz} Hz outside {MIN_SAMPLE_RATE_HZ}..={MAX_SAMPLE_RATE_HZ} Hz"),
        ))
    }
}

/// Whole frames in `audio` and the byte range they cover.
pub(super) fn whole_frames(audio: Range<u64>, block_align: u16) -> (u64, Range<u64>) {
    let align = u64::from(block_align.max(1));
    let frames = (audio.end.saturating_sub(audio.start)) / align;
    (frames, audio.start..audio.start + frames * align)
}

#[cfg(test)]
mod tests;
