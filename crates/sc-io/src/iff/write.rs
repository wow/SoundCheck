//! The bytes SoundCheck writes itself in a WAV or AIFF file: container and chunk headers, a
//! plain-PCM `fmt `, a plain AIFF `COMM`, and integer samples. Everything else in an output is
//! copied from the source.
//!
//! - `fmt `: `WAVEFORMATEX` without `cbSize`, 16 bytes, `wFormatTag` 0x0001
//!   (`WAVE_FORMAT_PCM`), the layout Microsoft's RIFF/WAVE specification (1991) defines for
//!   integer PCM and the one DJ hardware reads; `WAVE_FORMAT_EXTENSIBLE` is never written here.
//! - `COMM`: AIFF 1.3 (Apple, 1989), 18 bytes: `numChannels`, `numSampleFrames`, `sampleSize`
//!   and the sample rate as an 80-bit IEEE 754 extended value, written exactly.
//! - Samples: two's complement in `bits / 8` bytes, little-endian for WAV, big-endian for AIFF.

use super::extended::extended_from_sample_rate;

/// Payload bytes of a plain-PCM `fmt ` chunk.
pub const WAVE_FMT_PCM_BYTES: u32 = 16;
/// Payload bytes of a plain AIFF `COMM` chunk.
pub const AIFF_COMM_BYTES: u32 = 18;
/// Bytes before the sound data in an `SSND` payload: offset and block size, both 0.
pub const SSND_FIELDS_BYTES: u32 = 8;

/// The container an output is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutContainer {
    /// `RIFF` ... `WAVE`, little-endian.
    RiffWave,
    /// `FORM` ... `AIFF`, big-endian.
    FormAiff,
}

impl OutContainer {
    /// Whether sizes and samples are big-endian.
    #[must_use]
    pub fn is_big_endian(self) -> bool {
        self == Self::FormAiff
    }
}

/// The 12-byte container header: magic, `form_size` (bytes after the first 8), form type.
#[must_use]
pub fn container_header(container: OutContainer, form_size: u32) -> [u8; 12] {
    let mut h = [0_u8; 12];
    match container {
        OutContainer::RiffWave => {
            h[..4].copy_from_slice(b"RIFF");
            h[4..8].copy_from_slice(&form_size.to_le_bytes());
            h[8..].copy_from_slice(b"WAVE");
        }
        OutContainer::FormAiff => {
            h[..4].copy_from_slice(b"FORM");
            h[4..8].copy_from_slice(&form_size.to_be_bytes());
            h[8..].copy_from_slice(b"AIFF");
        }
    }
    h
}

/// An 8-byte chunk header: id and payload size (without the pad byte).
#[must_use]
pub fn chunk_header(container: OutContainer, id: [u8; 4], size: u32) -> [u8; 8] {
    let mut h = [0_u8; 8];
    h[..4].copy_from_slice(&id);
    h[4..].copy_from_slice(&if container.is_big_endian() {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    h
}

/// A plain-PCM `fmt ` payload for `channels` x `bits` (a multiple of 8) at `sample_rate_hz`.
#[must_use]
pub fn wave_fmt_pcm(channels: u16, sample_rate_hz: u32, bits: u16) -> [u8; 16] {
    let block_align = channels.saturating_mul(bits / 8);
    let byte_rate = sample_rate_hz.saturating_mul(u32::from(block_align));
    let mut p = [0_u8; 16];
    p[..2].copy_from_slice(&1_u16.to_le_bytes());
    p[2..4].copy_from_slice(&channels.to_le_bytes());
    p[4..8].copy_from_slice(&sample_rate_hz.to_le_bytes());
    p[8..12].copy_from_slice(&byte_rate.to_le_bytes());
    p[12..14].copy_from_slice(&block_align.to_le_bytes());
    p[14..].copy_from_slice(&bits.to_le_bytes());
    p
}

/// A plain AIFF `COMM` payload.
#[must_use]
pub fn aiff_comm(channels: u16, frames: u32, bits: u16, sample_rate_hz: u32) -> [u8; 18] {
    let mut p = [0_u8; 18];
    p[..2].copy_from_slice(&channels.to_be_bytes());
    p[2..6].copy_from_slice(&frames.to_be_bytes());
    p[6..8].copy_from_slice(&bits.to_be_bytes());
    p[8..].copy_from_slice(&extended_from_sample_rate(sample_rate_hz));
    p
}

/// Encodes `samples` (values at `bits` = 16 or 24 bits) into `out`, which is cleared first.
/// Allocation-free once `out` has grown to the block size.
pub fn encode_samples(samples: &[i32], bits: u16, big_endian: bool, out: &mut Vec<u8>) {
    out.clear();
    match (bits, big_endian) {
        (16, false) => encode::<2, false>(samples, out),
        (16, true) => encode::<2, true>(samples, out),
        (_, false) => encode::<3, false>(samples, out),
        (_, true) => encode::<3, true>(samples, out),
    }
}

fn encode<const W: usize, const BIG: bool>(samples: &[i32], out: &mut Vec<u8>) {
    out.resize(samples.len() * W, 0);
    for (bytes, s) in out.as_chunks_mut::<W>().0.iter_mut().zip(samples) {
        if BIG {
            bytes.copy_from_slice(&s.to_be_bytes()[4 - W..]);
        } else {
            bytes.copy_from_slice(&s.to_le_bytes()[..W]);
        }
    }
}

#[cfg(test)]
mod tests;
