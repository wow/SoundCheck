//! Whether an odd-length chunk is followed by its pad byte (Microsoft RIFF 1991 and AIFF 1.3
//! both require one; some writers omit it).
//!
//! A zero byte is a pad (no chunk id starts with 0). Otherwise both readings are followed
//! along the chain of chunk headers that would come next: "pad present" continues one byte
//! after the payload, "pad missing" right after it. A reading is `Exact` when every header on
//! the way has a valid id and a size that fits in the file, and the chain lands exactly on the
//! container end or the end of the file (or stays valid for [`MAX_DEPTH`] headers); it is
//! `Truncated` when the chain ends in a header whose size fits the declared container but not
//! the file (a file cut short). The better reading wins; on a tie, or when neither reading
//! works, the pad is present, as the specifications say. Odd chunks inside the lookahead are
//! tried both ways too (pad first), within a budget of [`MAX_PROBES`] headers per decision.

use std::io::{Read, Seek};

use sc_core::Result;

use super::Source;
use super::walk::{Header, Layout, read_header};

/// Headers followed before a chain counts as consistent.
pub(super) const MAX_DEPTH: u32 = 8;
/// Headers read at most for one pad decision.
pub(super) const MAX_PROBES: u32 = 256;

/// How well a reading of the bytes after a payload continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Landing {
    /// An invalid header, a size beyond the container and the file, or stray bytes.
    None,
    /// Valid headers ending in one whose size runs past the end of the file but not past the
    /// declared container end.
    Truncated,
    /// Valid headers landing exactly on the container end or the end of the file.
    Exact,
}

/// The pad byte after an odd payload ending at `end` (before the end of the file), or `None`
/// when the bytes say it is missing.
pub(super) fn decide<R: Read + Seek>(
    src: &mut Source<'_, R>,
    layout: &Layout<'_>,
    end: u64,
) -> Result<Option<u8>> {
    let mut byte = [0_u8; 1];
    src.read_at(end, &mut byte)?;
    if byte[0] == 0 {
        return Ok(Some(0));
    }
    let mut budget = MAX_PROBES;
    let present = probe(src, layout, end + 1, 0, &mut budget)?;
    if present == Landing::Exact {
        return Ok(Some(byte[0]));
    }
    let missing = probe(src, layout, end, 0, &mut budget)?;
    Ok((present >= missing).then_some(byte[0]))
}

/// Follows the chain of headers from `at`.
fn probe<R: Read + Seek>(
    src: &mut Source<'_, R>,
    layout: &Layout<'_>,
    at: u64,
    depth: u32,
    budget: &mut u32,
) -> Result<Landing> {
    if at == layout.container_end || at == src.len || depth == MAX_DEPTH {
        return Ok(Landing::Exact);
    }
    if at > src.len || src.len - at < 8 || *budget == 0 {
        return Ok(Landing::None);
    }
    *budget -= 1;
    let Some(Header { size, .. }) = read_header(src, layout, at)? else {
        return Ok(Landing::None);
    };
    let next = (at + 8).saturating_add(size);
    if next > src.len {
        return Ok(if next <= layout.container_end {
            Landing::Truncated
        } else {
            Landing::None
        });
    }
    if size % 2 == 1 && next < src.len {
        let padded = probe(src, layout, next + 1, depth + 1, budget)?;
        if padded == Landing::Exact {
            return Ok(padded);
        }
        let unpadded = probe(src, layout, next, depth + 1, budget)?;
        return Ok(padded.max(unpadded));
    }
    probe(src, layout, next, depth + 1, budget)
}
