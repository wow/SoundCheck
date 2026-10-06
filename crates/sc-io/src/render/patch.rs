//! Position and loudness fields of carried chunks, rewritten in place; every other byte of the
//! payload is kept. Layouts:
//! - `cue ` (Microsoft RIFF 1991, section 3): `dwCuePoints`, then 24-byte points whose
//!   `dwPosition` (+4) and `dwSampleOffset` (+20) are sample positions.
//! - `smpl` (same document): loop count at byte 28, 24-byte loops from byte 36 whose `dwStart`
//!   (+8) and `dwEnd` (+12) are sample positions.
//! - `MARK` (AIFF 1.3): `numMarkers` (u16), then markers of id (i16), position (u32) and a
//!   Pascal string name padded to an even length.
//! - `fact` (Microsoft RIFF 1991): `dwSampleLength` at byte 0.
//! - `bext` (EBU Tech 3285 v2, 2011): `TimeReference` (u64, samples since midnight) at byte
//!   338, `Version` (u16) at 346, the five loudness fields at 412..422 (reserved, zero, in
//!   versions 0 and 1, so an upgrade to version 2 writes them there).
//!
//! A head trim of `trim` frames moves every position `p` to `max(p - trim, 0)`; `bext`
//! `TimeReference` grows by `trim`, since it names the time of the first sample. A sampler
//! loop that starts inside the trimmed head (`dwStart < trim`, which includes every loop
//! ending there) would be cut or shortened, and the render is refused.
//!
//! `bext` loudness: given loudness upgrades any version to 2 and writes the five fields. A
//! gain without loudness makes the fields of a version 2 `bext` "not measured" (7FFFh) so they
//! never go stale; a version 0 or 1 `bext` has none, so its bytes stay as they are.
//! Multi-byte fields are little-endian in RIFF and big-endian in AIFF.

use sc_core::BextLoudness;

/// Why a payload could not be patched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// The payload is shorter than its own counts say.
    Malformed(&'static str),
    /// A sampler loop starts inside the trimmed head.
    LoopInTrim {
        /// The loop's `dwStart`, samples.
        start: u32,
        /// The loop's `dwEnd`, samples.
        end: u32,
    },
}

/// `bext` byte offset of `TimeReference`.
pub const BEXT_TIME_REFERENCE: usize = 338;
/// `bext` byte offset of `Version`.
pub const BEXT_VERSION: usize = 346;
/// `bext` byte offset of the loudness fields.
pub const BEXT_LOUDNESS: usize = 412;
/// Smallest `bext` payload: every fixed field up to the coding history (Tech 3285 v2).
pub const BEXT_MIN_BYTES: usize = 602;

fn read_u32(p: &[u8], at: usize, big: bool) -> Option<u32> {
    let b: [u8; 4] = p.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(if big {
        u32::from_be_bytes(b)
    } else {
        u32::from_le_bytes(b)
    })
}

/// Moves the position at `at` back by `trim`, clamping at 0.
fn shift_at(p: &mut [u8], at: usize, trim: u64, big: bool) -> Result<(), PatchError> {
    let v = read_u32(p, at, big).ok_or(PatchError::Malformed("position past the payload"))?;
    // Never larger than the value it came from.
    let shifted = u32::try_from(u64::from(v).saturating_sub(trim)).unwrap_or(0);
    let bytes = if big {
        shifted.to_be_bytes()
    } else {
        shifted.to_le_bytes()
    };
    p[at..at + 4].copy_from_slice(&bytes);
    Ok(())
}

/// Number of fixed-size records after a header, checked against the payload length.
fn records(p: &[u8], count: u32, start: usize, size: usize) -> Result<usize, PatchError> {
    let n = usize::try_from(count).map_err(|_| PatchError::Malformed("count too large"))?;
    let end = n
        .checked_mul(size)
        .and_then(|b| b.checked_add(start))
        .ok_or(PatchError::Malformed("count too large"))?;
    if end > p.len() {
        return Err(PatchError::Malformed("count runs past the payload"));
    }
    Ok(n)
}

/// `cue `: shifts `dwPosition` and `dwSampleOffset` of every point.
///
/// # Errors
/// [`PatchError::Malformed`] when the point count runs past the payload.
pub fn shift_cue(p: &mut [u8], trim: u64) -> Result<(), PatchError> {
    let count = read_u32(p, 0, false).ok_or(PatchError::Malformed("cue shorter than 4 bytes"))?;
    for i in 0..records(p, count, 4, 24)? {
        shift_at(p, 4 + 24 * i + 4, trim, false)?;
        shift_at(p, 4 + 24 * i + 20, trim, false)?;
    }
    Ok(())
}

/// `smpl`: shifts `dwStart` and `dwEnd` of every loop.
///
/// # Errors
/// [`PatchError::LoopInTrim`] when a loop starts or ends before `trim` (with `trim > 0`);
/// [`PatchError::Malformed`] when the loop count runs past the payload.
pub fn shift_smpl(p: &mut [u8], trim: u64) -> Result<(), PatchError> {
    let count =
        read_u32(p, 28, false).ok_or(PatchError::Malformed("smpl shorter than 36 bytes"))?;
    if p.len() < 36 {
        return Err(PatchError::Malformed("smpl shorter than 36 bytes"));
    }
    let n = records(p, count, 36, 24)?;
    for i in 0..n {
        let start = read_u32(p, 36 + 24 * i + 8, false)
            .ok_or(PatchError::Malformed("loop past the payload"))?;
        let end = read_u32(p, 36 + 24 * i + 12, false)
            .ok_or(PatchError::Malformed("loop past the payload"))?;
        if trim > 0 && (u64::from(start) < trim || u64::from(end) <= trim) {
            return Err(PatchError::LoopInTrim { start, end });
        }
    }
    for i in 0..n {
        shift_at(p, 36 + 24 * i + 8, trim, false)?;
        shift_at(p, 36 + 24 * i + 12, trim, false)?;
    }
    Ok(())
}

/// `MARK`: shifts every marker position.
///
/// # Errors
/// [`PatchError::Malformed`] when a marker runs past the payload.
pub fn shift_mark(p: &mut [u8], trim: u64) -> Result<(), PatchError> {
    let count = match p.get(..2) {
        Some(c) => u16::from_be_bytes([c[0], c[1]]),
        None => return Err(PatchError::Malformed("MARK shorter than 2 bytes")),
    };
    let mut at = 2_usize;
    for _ in 0..count {
        shift_at(p, at + 2, trim, true)?;
        let name = *p
            .get(at + 6)
            .ok_or(PatchError::Malformed("marker name past the payload"))?;
        at += 6 + (1 + usize::from(name)).next_multiple_of(2);
        if at > p.len() {
            return Err(PatchError::Malformed("marker name past the payload"));
        }
    }
    Ok(())
}

/// `fact`: sets `dwSampleLength` to `frames`.
///
/// # Errors
/// [`PatchError::Malformed`] when the payload is shorter than 4 bytes or `frames` does not fit
/// 32 bits.
pub fn set_fact(p: &mut [u8], frames: u64) -> Result<(), PatchError> {
    let frames = u32::try_from(frames).map_err(|_| PatchError::Malformed("frames past 32 bits"))?;
    let field = p
        .get_mut(..4)
        .ok_or(PatchError::Malformed("fact shorter than 4 bytes"))?;
    field.copy_from_slice(&frames.to_le_bytes());
    Ok(())
}

/// What a render does to the loudness fields of a `bext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BextUpdate {
    /// Leave them (no gain, no loudness given).
    Keep,
    /// Version 2 and these ten bytes (loudness given).
    Set([u8; 10]),
    /// After a gain without loudness: 7FFFh in every field, only when the chunk is version 2
    /// or later (earlier versions carry no loudness to go stale).
    ClearIfVersion2,
}

/// The `bext` Version, when the payload reaches it.
#[must_use]
pub fn bext_version(p: &[u8]) -> Option<u16> {
    p.get(BEXT_VERSION..BEXT_VERSION + 2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
}

/// `bext`: `TimeReference` plus `trim`, loudness per `update`. Returns whether anything had
/// to change; when nothing does, the payload is untouched and its length is not checked (a
/// short or malformed `bext` is carried as it is).
///
/// # Errors
/// [`PatchError::Malformed`] when a change is needed and the payload is shorter than
/// [`BEXT_MIN_BYTES`].
pub fn patch_bext(p: &mut [u8], trim: u64, update: BextUpdate) -> Result<bool, PatchError> {
    let fields = match update {
        BextUpdate::Keep => None,
        BextUpdate::Set(bytes) => Some(bytes),
        BextUpdate::ClearIfVersion2 => bext_version(p)
            .filter(|v| *v >= 2)
            .map(|_| BextLoudness::ALL_UNMEASURED.to_le_bytes()),
    };
    if trim == 0 && fields.is_none() {
        return Ok(false);
    }
    if p.len() < BEXT_MIN_BYTES {
        return Err(PatchError::Malformed("bext shorter than 602 bytes"));
    }
    let mut t = [0_u8; 8];
    t.copy_from_slice(&p[BEXT_TIME_REFERENCE..BEXT_TIME_REFERENCE + 8]);
    let time = u64::from_le_bytes(t).saturating_add(trim);
    p[BEXT_TIME_REFERENCE..BEXT_TIME_REFERENCE + 8].copy_from_slice(&time.to_le_bytes());
    if let Some(fields) = fields {
        p[BEXT_VERSION..BEXT_VERSION + 2].copy_from_slice(&2_u16.to_le_bytes());
        p[BEXT_LOUDNESS..BEXT_LOUDNESS + 10].copy_from_slice(&fields);
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
