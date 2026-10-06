//! Shifting a CUESHEET block (RFC 9639 section 8.7) by a head trim.
//!
//! Layout: a 128-byte media catalog number, the 64-bit lead-in sample count, a flags byte
//! (bit 7: CD-DA), 258 reserved bytes and the track count (396 bytes); per track a 64-bit
//! offset (samples from the start of the stream), the track number, a 12-byte ISRC, a flags
//! byte, 13 reserved bytes and the index point count (36 bytes); per index point a 64-bit
//! offset relative to its track, the index number and 3 reserved bytes (12 bytes). All
//! big-endian.
//!
//! Under a head trim of `n` samples every index point's absolute position (track offset +
//! index offset) moves by `-n` and clamps at 0; each track's offset becomes its first index
//! point's new absolute position and its index offsets are rebased on it (so a pregap cut by
//! the trim shrinks instead of the track start sliding); a track without index points keeps
//! its offset minus `n`, clamped; the lead-out track, which RFC 9639 makes the last track
//! (numbered 170 on CD-DA, 255 otherwise, but found by its place), gets the new total sample
//! count. Every other byte is kept. A CD-DA sheet requires every offset
//! to be a multiple of 588 samples (one CD sector), so a trim that is not is refused rather
//! than written into an invalid sheet.

/// Samples per CD sector (1/75 s at 44,100 Hz).
pub const CD_SECTOR_SAMPLES: u64 = 588;

const HEADER_BYTES: usize = 396;
const TRACK_BYTES: usize = 36;
const INDEX_BYTES: usize = 12;
const CDDA_FLAG_AT: usize = 136;

/// Why a CUESHEET cannot be shifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuesheetError {
    /// The payload does not parse; the text says what is wrong.
    Malformed(&'static str),
    /// The sheet is CD-DA and the trim is not a multiple of [`CD_SECTOR_SAMPLES`].
    CdDaAlignment,
}

fn be64(p: &[u8], at: usize) -> Result<u64, CuesheetError> {
    let b = p
        .get(at..at + 8)
        .ok_or(CuesheetError::Malformed("a track runs past the block"))?;
    let mut a = [0_u8; 8];
    a.copy_from_slice(b);
    Ok(u64::from_be_bytes(a))
}

/// Shifts the CUESHEET payload `p` in place for a head trim of `trim` samples leaving
/// `total_out` samples; returns whether a byte changed.
///
/// # Errors
/// [`CuesheetError::Malformed`] for a payload shorter than its track list or with index
/// points out of order; [`CuesheetError::CdDaAlignment`] for a CD-DA sheet and a trim that is
/// not a whole number of sectors. `p` is unchanged on error.
pub fn shift_cuesheet(p: &mut [u8], trim: u64, total_out: u64) -> Result<bool, CuesheetError> {
    let tracks = *p
        .get(HEADER_BYTES - 1)
        .ok_or(CuesheetError::Malformed("shorter than its header"))?;
    if p[CDDA_FLAG_AT] & 0x80 != 0 && !trim.is_multiple_of(CD_SECTOR_SAMPLES) {
        return Err(CuesheetError::CdDaAlignment);
    }
    // Every new value is computed before any byte is written, so an error leaves `p` as it was.
    let mut writes: Vec<(usize, u64)> = Vec::new();
    let mut pos = HEADER_BYTES;
    for t in 0..tracks {
        let offset = be64(p, pos)?;
        let count = usize::from(
            *p.get(pos + TRACK_BYTES - 1)
                .ok_or(CuesheetError::Malformed("a track runs past the block"))?,
        );
        let mut absolute = Vec::with_capacity(count);
        for k in 0..count {
            let index = be64(p, pos + TRACK_BYTES + INDEX_BYTES * k)?;
            let abs = offset.checked_add(index).ok_or(CuesheetError::Malformed(
                "an index point lies past 2^64 samples",
            ))?;
            absolute.push(abs.saturating_sub(trim));
        }
        let new_offset = if t + 1 == tracks {
            total_out
        } else {
            absolute
                .first()
                .copied()
                .unwrap_or(offset.saturating_sub(trim))
        };
        writes.push((pos, new_offset));
        for (k, abs) in absolute.iter().enumerate() {
            let rel = abs
                .checked_sub(new_offset)
                .ok_or(CuesheetError::Malformed("index points are out of order"))?;
            writes.push((pos + TRACK_BYTES + INDEX_BYTES * k, rel));
        }
        pos += TRACK_BYTES + INDEX_BYTES * count;
    }
    let mut changed = false;
    for (at, v) in writes {
        let bytes = v.to_be_bytes();
        changed |= p[at..at + 8] != bytes;
        p[at..at + 8].copy_from_slice(&bytes);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
