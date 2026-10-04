//! Whether an odd-length chunk is followed by its pad byte (Microsoft RIFF 1991 and AIFF 1.3
//! both require one; some writers omit it).
//!
//! A zero byte is a pad (no chunk id starts with 0). Otherwise both readings are followed
//! along the chain of chunk headers that would come next: "pad present" continues one byte
//! after the payload, "pad missing" right after it. A reading is `Exact` when every header on
//! the way has a valid id and a size that fits before the end of the walk (the end of the file
//! or the start of an `ID3v1` tag), and the chain lands exactly on the container end or the
//! end of the walk (or stays valid for [`MAX_DEPTH`] headers); it is `Truncated` when the chain
//! ends in a header whose size fits the declared container but not the walk (a file cut
//! short). The better reading wins; on a tie, or when neither reading works, the pad is
//! present, as the specifications say. Odd chunks inside the lookahead are tried both ways too
//! (pad first), within a budget of [`MAX_PROBES`] headers per decision and the walk's remaining
//! budget; with that exhausted, the pad is assumed present.

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

/// The pad byte after an odd payload ending at `end` (before the end of the walk), or `None`
/// when the bytes say it is missing. Probe headers read are taken from `walk_budget`.
pub(super) fn decide<R: Read + Seek>(
    src: &mut Source<'_, R>,
    layout: &Layout<'_>,
    end: u64,
    walk_budget: &mut u64,
) -> Result<Option<u8>> {
    let mut byte = [0_u8; 1];
    src.read_at(end, &mut byte)?;
    if byte[0] == 0 {
        return Ok(Some(0));
    }
    let allowed = u32::try_from((*walk_budget).min(u64::from(MAX_PROBES))).unwrap_or(MAX_PROBES);
    if allowed == 0 {
        return Ok(Some(byte[0]));
    }
    let mut budget = allowed;
    let present = probe(src, layout, end + 1, 0, &mut budget)?;
    let decision = if present == Landing::Exact {
        Some(byte[0])
    } else {
        let missing = probe(src, layout, end, 0, &mut budget)?;
        (present >= missing).then_some(byte[0])
    };
    *walk_budget -= u64::from(allowed - budget);
    Ok(decision)
}

/// Follows the chain of headers from `at`.
fn probe<R: Read + Seek>(
    src: &mut Source<'_, R>,
    layout: &Layout<'_>,
    at: u64,
    depth: u32,
    budget: &mut u32,
) -> Result<Landing> {
    let end = layout.walk_end;
    if at == layout.container_end || at == end || depth == MAX_DEPTH {
        return Ok(Landing::Exact);
    }
    if at > end || end - at < 8 || *budget == 0 {
        return Ok(Landing::None);
    }
    *budget -= 1;
    let Some(Header { size, .. }) = read_header(src, layout, at)? else {
        return Ok(Landing::None);
    };
    let next = (at + 8).saturating_add(size);
    if next > end {
        return Ok(if next <= layout.container_end {
            Landing::Truncated
        } else {
            Landing::None
        });
    }
    if size % 2 == 1 && next < end {
        let padded = probe(src, layout, next + 1, depth + 1, budget)?;
        if padded == Landing::Exact {
            return Ok(padded);
        }
        let unpadded = probe(src, layout, next, depth + 1, budget)?;
        return Ok(padded.max(unpadded));
    }
    probe(src, layout, next, depth + 1, budget)
}
