//! Rebuilding a SEEKTABLE block (RFC 9639 section 8.5) for re-encoded frames.
//!
//! A seek point is 18 bytes: the 64-bit sample number of a frame's first sample, the 64-bit
//! byte offset of that frame from the first frame, and the frame's 16-bit sample count; a
//! placeholder has sample number `0xFFFFFFFFFFFFFFFF` (offset and count undefined, written as
//! 0). Points are sorted by sample number, unique, placeholders last.
//!
//! The rebuilt table has the source table's whole points (bytes after the last whole point of a
//! malformed table are dropped), so the block keeps its size and can be written before the
//! frames: as many real points as the source had (at most one per frame,
//! the rest become placeholders), at evenly spaced frames (`k * frames / real` for `k` in
//! `0..real`, so the first frame is always a point), then the placeholders.

/// Bytes per seek point.
pub const SEEK_POINT_BYTES: usize = 18;

/// The sample number of a placeholder point.
pub const PLACEHOLDER: u64 = u64::MAX;

/// How many real and placeholder points a table has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeekShape {
    /// Points with a sample number.
    pub real: usize,
    /// Placeholder points.
    pub placeholders: usize,
}

impl SeekShape {
    /// Counts the whole points of a SEEKTABLE payload; bytes after the last whole point (a
    /// length that is not a multiple of [`SEEK_POINT_BYTES`]) are not a point and are left out.
    #[must_use]
    pub fn of(payload: &[u8]) -> Self {
        let (points, _) = payload.as_chunks::<SEEK_POINT_BYTES>();
        let placeholders = points
            .iter()
            .filter(|p| p[..8] == PLACEHOLDER.to_be_bytes())
            .count();
        Self {
            real: points.len() - placeholders,
            placeholders,
        }
    }

    /// Payload length, bytes.
    #[must_use]
    pub fn bytes(&self) -> usize {
        (self.real + self.placeholders) * SEEK_POINT_BYTES
    }
}

/// The table of `shape`'s size for frames of `block` samples (the last one shorter) starting
/// at `frame_offsets` (bytes from the first frame), `total` samples in all.
#[must_use]
pub fn build(shape: SeekShape, frame_offsets: &[u64], block: u64, total: u64) -> Vec<u8> {
    let frames = frame_offsets.len();
    let real = shape.real.min(frames);
    let mut p = Vec::with_capacity(shape.bytes());
    for k in 0..real {
        // k < real <= frames, so the index stays below `frames` and the product fits u128.
        let idx = usize::try_from(k as u128 * frames as u128 / real as u128).unwrap_or(0);
        let sample = idx as u64 * block;
        let samples = u16::try_from(block.min(total.saturating_sub(sample))).unwrap_or(u16::MAX);
        p.extend_from_slice(&sample.to_be_bytes());
        p.extend_from_slice(&frame_offsets[idx].to_be_bytes());
        p.extend_from_slice(&samples.to_be_bytes());
    }
    for _ in real..shape.real + shape.placeholders {
        p.extend_from_slice(&PLACEHOLDER.to_be_bytes());
        p.extend_from_slice(&[0; 10]);
    }
    p
}

#[cfg(test)]
mod tests;
