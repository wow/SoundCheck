//! The head cut decided before the render: [`snap_head_cut`] gives the cut a render makes for a
//! requested cut (moved up to 1 ms earlier to the quietest frame, as [`super::head`]
//! describes), so a caller can plan every position that depends on the cut (the frame count,
//! bar 1 in the output, the tags that record them) from the cut actually made, and then render
//! exactly that cut with [`RenderRequest::trim_snapped_from`] set.
//!
//! The snap is not idempotent: snapping the snapped cut again may move it further back (when
//! the audio keeps getting quieter towards the start). So a render whose request carries an
//! already snapped cut does not snap; it checks that the cut lies in the window the snap of the
//! original request searches, and otherwise renders exactly as a render of the original
//! request does (same cut, fade, dither seed, report), byte for byte.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, Result};

use super::head::head_snap_frames;
use super::{audio, flac, io_error};
use crate::iff;
use crate::txn::{TagFamily, tag_family};

/// The head cut a render of `path` makes when asked to cut `requested_frames` frames: the
/// quietest frame at most 1 ms (rounded to whole frames) before it, never after it; 0 for 0.
/// Only the frames up to the requested cut are decoded.
///
/// # Errors
/// [`Error::UnsupportedFormat`] for anything but WAV, RF64, AIFF, AIFF-C and FLAC, or a file
/// whose headers do not read as one; [`Error::InvalidArgument`] for a cut that leaves no audio;
/// [`Error::Corrupt`] for audio that does not decode; [`Error::Io`].
pub fn snap_head_cut(path: &Path, requested_frames: u64) -> Result<u64> {
    if requested_frames == 0 {
        return Ok(0);
    }
    let cancel = AtomicBool::new(false);
    let mut src = File::open(path).map_err(|e| io_error(path, e))?;
    let (frames, snapped) = match tag_family(path)? {
        TagFamily::Id3 => {
            let header = iff::read_header(&mut src, path)?;
            let format = &header.format;
            check_leaves_audio(path, requested_frames, format.frames)?;
            let snapped = audio::snapped_trim(&mut src, path, format, requested_frames, &cancel)?;
            (format.frames, snapped)
        }
        TagFamily::Vorbis => {
            let layout = crate::flac::read_layout(&mut src, path)?;
            let frames = flac::frames_in(&mut src, path, &layout, &cancel)?;
            check_leaves_audio(path, requested_frames, frames)?;
            let snapped =
                flac::snapped_trim(&mut src, path, &layout, requested_frames, frames, &cancel)?;
            (frames, snapped)
        }
    };
    tracing::debug!(
        path = %path.display(),
        stage = "snap-head-cut",
        requested_frames,
        snapped,
        frames,
        "snapped"
    );
    Ok(snapped)
}

/// [`Error::InvalidArgument`] when cutting `trim_frames` of `frames` leaves no audio.
pub(super) fn check_leaves_audio(path: &Path, trim_frames: u64, frames: u64) -> Result<()> {
    if trim_frames >= frames {
        return Err(Error::InvalidArgument(format!(
            "{}: a head trim of {trim_frames} samples leaves no audio ({frames} frames)",
            path.display(),
        )));
    }
    Ok(())
}

/// The cut a render of `req` makes and the cut it reports as requested: the request's cut and
/// its original request when it is already snapped (checked to lie in that request's window),
/// else the snap of the request's cut, which `snap` computes.
///
/// # Errors
/// [`Error::InvalidArgument`] for an already snapped cut after its request or more than
/// [`head_snap_frames`] before it; whatever `snap` returns.
pub(super) fn head_cut(
    req: &RenderRequest,
    sample_rate_hz: u32,
    snap: impl FnOnce() -> Result<u64>,
) -> Result<(u64, u64)> {
    let Some(requested) = req.trim_snapped_from else {
        return Ok((snap()?, req.trim_frames));
    };
    let window = head_snap_frames(sample_rate_hz);
    let earliest = requested.saturating_sub(window);
    if req.trim_frames > requested || req.trim_frames < earliest {
        return Err(Error::InvalidArgument(format!(
            "a head cut of {} frames is not a snap of {requested} frames (the snap moves a cut \
             back by at most {window} frames, never later)",
            req.trim_frames
        )));
    }
    Ok((req.trim_frames, requested))
}

#[cfg(test)]
mod tests;
