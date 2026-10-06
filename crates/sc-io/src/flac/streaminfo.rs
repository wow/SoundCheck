//! STREAMINFO (RFC 9639 section 8.2): 34 bytes, big-endian, bit-packed.
//!
//! | bits | field |
//! |---|---|
//! | 16 | minimum block size, samples (the last block of the stream excluded) |
//! | 16 | maximum block size, samples |
//! | 24 | minimum frame size, bytes (0 = unknown) |
//! | 24 | maximum frame size, bytes (0 = unknown) |
//! | 20 | sample rate, Hz |
//! | 3 | channels - 1 |
//! | 5 | bits per sample - 1 |
//! | 36 | total samples per channel (0 = unknown) |
//! | 128 | MD5 of the decoded samples (all zero = not computed) |

/// Length of a STREAMINFO payload, bytes.
pub const STREAMINFO_BYTES: usize = 34;

/// The fields of a STREAMINFO block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamInfo {
    /// Smallest block size, samples per channel (the last block excluded).
    pub min_block: u16,
    /// Largest block size, samples per channel.
    pub max_block: u16,
    /// Smallest frame, bytes (0 = unknown; 24 bits).
    pub min_frame: u32,
    /// Largest frame, bytes (0 = unknown; 24 bits).
    pub max_frame: u32,
    /// Sample rate, Hz (20 bits).
    pub sample_rate_hz: u32,
    /// Channels, 1..=8.
    pub channels: u8,
    /// Bits per sample, 1..=32 (RFC 9639 allows 4..=32).
    pub bits: u8,
    /// Samples per channel; 0 means unknown (36 bits).
    pub total_samples: u64,
    /// MD5 of the samples; all zero means it was not computed.
    pub md5: [u8; 16],
}

/// Largest total sample count the 36-bit field holds.
pub const MAX_TOTAL_SAMPLES: u64 = (1 << 36) - 1;

impl StreamInfo {
    /// Decodes a STREAMINFO payload.
    #[must_use]
    pub fn parse(p: &[u8; STREAMINFO_BYTES]) -> Self {
        let be24 = |at: usize| u32::from_be_bytes([0, p[at], p[at + 1], p[at + 2]]);
        let mut packed = [0_u8; 8];
        packed.copy_from_slice(&p[10..18]);
        let packed = u64::from_be_bytes(packed);
        let mut md5 = [0_u8; 16];
        md5.copy_from_slice(&p[18..34]);
        // The masks keep each field inside its target type.
        #[allow(clippy::cast_possible_truncation)]
        Self {
            min_block: u16::from_be_bytes([p[0], p[1]]),
            max_block: u16::from_be_bytes([p[2], p[3]]),
            min_frame: be24(4),
            max_frame: be24(7),
            sample_rate_hz: (packed >> 44) as u32 & 0xF_FFFF,
            channels: ((packed >> 41) & 0x7) as u8 + 1,
            bits: ((packed >> 36) & 0x1F) as u8 + 1,
            total_samples: packed & MAX_TOTAL_SAMPLES,
            md5,
        }
    }

    /// Encodes the payload. Fields wider than their bit width are truncated to it; callers
    /// keep them in range (frame sizes below 2^24, rate below 2^20, total below 2^36).
    #[must_use]
    pub fn to_bytes(&self) -> [u8; STREAMINFO_BYTES] {
        let mut p = [0_u8; STREAMINFO_BYTES];
        p[0..2].copy_from_slice(&self.min_block.to_be_bytes());
        p[2..4].copy_from_slice(&self.max_block.to_be_bytes());
        p[4..7].copy_from_slice(&self.min_frame.to_be_bytes()[1..]);
        p[7..10].copy_from_slice(&self.max_frame.to_be_bytes()[1..]);
        let packed = u64::from(self.sample_rate_hz & 0xF_FFFF) << 44
            | u64::from(self.channels.saturating_sub(1) & 0x7) << 41
            | u64::from(self.bits.saturating_sub(1) & 0x1F) << 36
            | (self.total_samples & MAX_TOTAL_SAMPLES);
        p[10..18].copy_from_slice(&packed.to_be_bytes());
        p[18..34].copy_from_slice(&self.md5);
        p
    }

    /// Whether the MD5 field holds a signature (all zero means none was computed).
    #[must_use]
    pub fn has_md5(&self) -> bool {
        self.md5 != [0; 16]
    }

    /// Bytes per sample in the MD5 input: `ceil(bits / 8)`.
    #[must_use]
    pub fn md5_bytes_per_sample(&self) -> usize {
        usize::from(self.bits).div_ceil(8)
    }
}

#[cfg(test)]
mod tests;
