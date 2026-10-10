//! Rendering a FLAC file with a new level (and an optional head trim) into a FLAC file that
//! keeps everything else (RFC 9639; see [`crate::flac`] for the pieces).
//!
//! [`apply_flac`] streams `source frames -> decode -> trim -> fade-in -> gain -> requantise ->
//! encode -> writer`, with memory bounded by one frame of audio and the list of frame offsets:
//!
//! - **Head cut**: snapped back by up to 1 ms to the quietest frame (unless the request says
//!   it already was) and faded in over 2 ms as for WAV/AIFF (see [`super::head`]); the
//!   CUESHEET shift and the new total use the cut actually made.
//! - **Around the stream**: `ID3v2` tags in front of `fLaC` and whatever follows the last frame
//!   (an `ID3v1` or `APEv2` tag) are carried byte for byte at the same ends of the output.
//! - **STREAMINFO** is rebuilt: block size 4096 (the minimum and the maximum; only the last
//!   frame is shorter, which RFC 9639 excludes from the minimum), the smallest and largest frame
//!   written, the sample rate and channels of the source, the output depth, the new total and
//!   the MD5 of the output samples.
//! - **SEEKTABLE** is rebuilt with as many points as the source table (real points on evenly
//!   spaced frames, placeholders kept as placeholders; see [`crate::flac::seektable`]); bytes
//!   after the last whole point of a malformed table go to the first PADDING block.
//! - **`VORBIS_COMMENT`** with tag edits ([`RenderRequest::tag_edits`], Vorbis field names): the
//!   vendor string and every field byte for byte and in order, a field with an edited name
//!   (case-insensitive) given the new value in place, the other edits appended once (see
//!   [`crate::flac::vorbis`]). With no comment block (none is created), several, or one that
//!   does not parse, every block is carried, the render goes on, and
//!   [`RenderReport::tags_not_added`] says why.
//! - **PADDING** is written as zeros; the first PADDING block shrinks by what the comment grew
//!   (down to empty) or grows by what it shrank, so the metadata keeps its length when it can.
//! - **CUESHEET** under a head trim: positions shift as [`crate::flac::cuesheet`] describes.
//! - **Carried byte for byte**, in source order: APPLICATION, PICTURE, unknown block types, and
//!   the blocks above when nothing changes them. The last-block flag is set on the last block.
//! - **Audio**: frames of 4096 samples at the requested depth (16 or 24) or the source's (16
//!   for 16 bits or fewer, else 24; a 24-bit source stays 24), with gain, rounding and dither
//!   as [`sc_dsp::requantise`] specifies. The source's frames are decoded exactly (CRCs checked)
//!   and, when STREAMINFO has an MD5 signature, checked against it.
//! - **Stale loudness tags**: after a gain change, `REPLAYGAIN_*` and `R128_*` fields that no
//!   edit replaces are carried and listed in [`RenderReport::stale_loudness_tags`].
//! - **Verified**: after writing, the output is walked again and its frames decoded with an
//!   independent decoder; the sample count, the BLAKE3 of the samples (against the hash taken
//!   while encoding) and the MD5 (against the STREAMINFO just written) must all match, or the
//!   output is removed and the render fails with [`Error::Corrupt`] naming the output.
//!
//! Refused, with no output file left behind: more than two channels
//! ([`Error::UnsupportedChannels`]); a sample rate other than 44,100 or 48,000 Hz
//! ([`Error::NotDjSafe`]); a sample after gain at or above +1.0 of full scale
//! ([`Error::WouldClip`]); a trim that leaves no audio or breaks a CD-DA cue sheet, an invalid
//! tag edit ([`Error::InvalidArgument`]); a source whose frames do not decode, do not match
//! their count or MD5, run past the total STREAMINFO declares or are followed by more frames,
//! or whose CUESHEET (under a trim) is malformed ([`Error::Corrupt`]); no `fLaC` marker ([`Error::UnsupportedFormat`]). A cancel flag,
//! checked once per frame, stops the render with [`Error::Cancelled`]. The output is created
//! new (`create_new`) and removed on any error or panic; I/O errors name the input when reading fails and the output when
//! writing fails.

mod plan;
mod write;

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, Result};

use super::audio::{Peaks, SEED_FRAMES, SeedRequest, check_cancel, seed_hasher, seed_of};
use super::head::{self, HeadSnap};
use super::{
    OutputGuard, RenderReport, check_full_scale, check_rate, check_request, io_error, output_bits,
};
use crate::flac::{FlacLayout, FlacPcm, read_layout, vorbis};
use plan::{PlanArgs, TagOutcome};

/// What the output holds and how it is made.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Target {
    /// Source bits per sample.
    bits_in: u16,
    /// Output bits per sample (16 or 24).
    bits_out: u16,
    /// Frames in the source.
    frames_in: u64,
    /// Frames removed from the start.
    trim_frames: u64,
    /// Frames written.
    frames_out: u64,
    /// Gain, dB.
    gain_db: f64,
    /// TPDF seed.
    seed: u64,
}

/// Renders the FLAC file `input` with `req` applied into the new file `output`; `cancel`
/// (checked once per frame) stops it.
///
/// # Errors
/// The refusals listed in the module documentation, [`Error::InvalidArgument`] for a request
/// that is not valid (gain not finite, depth other than 16 or 24), [`Error::Cancelled`],
/// [`Error::Io`] when reading or writing fails or `output` exists. On any error no output file
/// is left.
pub fn apply_flac(
    input: &Path,
    output: &Path,
    req: &RenderRequest,
    cancel: &AtomicBool,
) -> Result<RenderReport> {
    let (report, check) = render_flac_unverified(input, output, req, cancel)?;
    let guard = OutputGuard::new(output);
    check.run(output, cancel)?;
    guard.keep();
    Ok(report)
}

/// The verification [`apply_flac`] runs on its output, for a caller that verifies later (after
/// syncing the file): the metadata as planned, the STREAMINFO as written, and the frames
/// decoded by an independent decoder to the encoded samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlacCheck(write::Expected);

impl FlacCheck {
    /// Checks the file at `output`.
    ///
    /// # Errors
    /// [`Error::Corrupt`] naming `output` for any difference; [`Error::Io`];
    /// [`Error::Cancelled`].
    pub(crate) fn run(&self, output: &Path, cancel: &AtomicBool) -> Result<()> {
        write::verify(output, &self.0, cancel)
    }
}

/// [`apply_flac`] without the final verification, which the returned check performs. The
/// output is kept when this returns `Ok`; the caller removes it if the check fails.
///
/// # Errors
/// As for [`apply_flac`], verification failures excepted.
pub(crate) fn render_flac_unverified(
    input: &Path,
    output: &Path,
    req: &RenderRequest,
    cancel: &AtomicBool,
) -> Result<(RenderReport, FlacCheck)> {
    check_request(req)?;
    let edits = vorbis::edits_from(&req.tag_edits)?;
    let mut src = File::open(input).map_err(|e| io_error(input, e))?;
    let layout = read_layout(&mut src, input)?;
    let info = layout.streaminfo;
    let mut target = target(&mut src, input, &layout, req, cancel)?;
    let frames_in = target.frames_in;
    let (trim_frames, trim_requested_frames) =
        super::snap::head_cut(req, info.sample_rate_hz, || {
            snapped_trim(&mut src, input, &layout, req.trim_frames, frames_in, cancel)
        })?;
    target.trim_frames = trim_frames;
    target.frames_out = frames_in - target.trim_frames;
    let plan = plan::plan(
        &mut src,
        input,
        &layout,
        PlanArgs {
            trim_frames: target.trim_frames,
            frames_out: target.frames_out,
            edits: &edits,
            gain_changed: req.gain_db != 0.0,
        },
    )?;
    if req.gain_db > 0.0 {
        let peaks = peaks_after_trim(&mut src, input, &layout, target.trim_frames, cancel)?;
        check_full_scale(peaks, req.gain_db)?;
    }
    target.seed = dither_seed(&mut src, input, &layout, &target, cancel)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| io_error(output, e))?;
    let job = write::Job {
        input,
        output,
        layout: &layout,
        plan: &plan,
        target: &target,
        cancel,
    };
    let guard = OutputGuard::new(output);
    let (done, expected) = write::write(&mut src, file, &job)?;
    guard.keep();
    tracing::info!(
        path = %input.display(),
        output = %output.display(),
        gain_db = req.gain_db,
        trim_frames = target.trim_frames,
        trim_requested_frames,
        bits = target.bits_out,
        frames = target.frames_out,
        bytes = done.output_bytes,
        tags_added = matches!(plan.tags, TagOutcome::Edited(_)),
        "rendered"
    );
    let (vorbis_edit, tags_not_added) = match &plan.tags {
        TagOutcome::Edited(summary) => (Some(*summary), None),
        TagOutcome::NotAdded(reason) => (None, Some(reason.clone())),
        TagOutcome::NotRequested => (None, None),
    };
    let report = RenderReport {
        tags_added: vorbis_edit.is_some(),
        tags_not_added,
        tag_edit: None,
        vorbis_edit,
        frames_in,
        frames_out: target.frames_out,
        trim_frames: target.trim_frames,
        trim_requested_frames,
        sample_rate_hz: info.sample_rate_hz,
        channels: u16::from(info.channels),
        bits_out: target.bits_out,
        exact: done.exact,
        dithered: done.dithered,
        samples_saturated: done.saturated,
        pcm_hash: done.pcm_hash,
        stale_loudness_tags: plan.stale_loudness,
        blocks: plan.records,
        leading_bytes: layout.marker_offset,
        trailing_bytes: done.trailing_bytes,
        output_bytes: done.output_bytes,
    };
    Ok((report, FlacCheck(expected)))
}

/// The output's shape (the seed is set later), or why the source cannot be rendered DJ-safe.
fn target(
    src: &mut File,
    input: &Path,
    layout: &FlacLayout,
    req: &RenderRequest,
    cancel: &AtomicBool,
) -> Result<Target> {
    let info = layout.streaminfo;
    if info.channels > 2 {
        return Err(Error::UnsupportedChannels {
            path: input.to_path_buf(),
            channels: usize::from(info.channels),
        });
    }
    check_rate(input, info.sample_rate_hz)?;
    let frames_in = frames_in(src, input, layout, cancel)?;
    super::snap::check_leaves_audio(input, req.trim_frames, frames_in)?;
    let bits_in = u16::from(info.bits);
    Ok(Target {
        bits_in,
        bits_out: output_bits(req, false, bits_in),
        frames_in,
        trim_frames: req.trim_frames,
        frames_out: frames_in - req.trim_frames,
        gain_db: req.gain_db,
        seed: 0,
    })
}

/// The source's frame count: STREAMINFO's total, or, when it declares none, the frames counted
/// by decoding them all.
pub(super) fn frames_in(
    src: &mut File,
    path: &Path,
    layout: &FlacLayout,
    cancel: &AtomicBool,
) -> Result<u64> {
    match layout.streaminfo.total_samples {
        0 => count_frames(src, path, layout, cancel),
        n => Ok(n),
    }
}

/// Decodes every frame of a source whose STREAMINFO does not declare the total.
fn count_frames(
    src: &mut File,
    path: &Path,
    layout: &FlacLayout,
    cancel: &AtomicBool,
) -> Result<u64> {
    let mut pcm = FlacPcm::open(src, path, layout, false)?;
    let mut block = Vec::new();
    loop {
        check_cancel(cancel)?;
        if pcm.next_block(&mut block)? == 0 {
            break;
        }
    }
    Ok(pcm.finish()?.frames)
}

/// The head cut actually made for a requested cut of `requested` frames of a source of
/// `frames_in` frames (see [`super::head`]).
pub(super) fn snapped_trim(
    src: &mut File,
    path: &Path,
    layout: &FlacLayout,
    requested: u64,
    frames_in: u64,
    cancel: &AtomicBool,
) -> Result<u64> {
    if requested == 0 || requested >= frames_in {
        return Ok(requested);
    }
    let info = &layout.streaminfo;
    let mut snap = HeadSnap::new(requested, info.sample_rate_hz, u16::from(info.channels));
    let mut pcm = FlacPcm::open(src, path, layout, false)?;
    let mut block = Vec::new();
    while snap.wants_more() {
        check_cancel(cancel)?;
        if pcm.next_block(&mut block)? == 0 {
            break;
        }
        snap.push(&block, head::int_magnitude);
    }
    Ok(snap.finish())
}

/// The signed peaks of the samples after the trim, as fractions of full scale.
fn peaks_after_trim(
    src: &mut File,
    path: &Path,
    layout: &FlacLayout,
    trim_frames: u64,
    cancel: &AtomicBool,
) -> Result<Peaks> {
    let channels = u64::from(layout.streaminfo.channels);
    let mut pcm = FlacPcm::open(src, path, layout, false)?;
    let mut block = Vec::new();
    let mut skip = trim_frames.saturating_mul(channels);
    let (mut max, mut min) = (0_i32, 0_i32);
    loop {
        check_cancel(cancel)?;
        if pcm.next_block(&mut block)? == 0 {
            break;
        }
        let start = usize::try_from(skip.min(block.len() as u64)).unwrap_or(0);
        skip -= start as u64;
        for x in &block[start..] {
            max = max.max(*x);
            min = min.min(*x);
        }
    }
    pcm.finish()?;
    // The full-scale divisor is a power of two, so the quotients are exact.
    let full = 2_f64.powi(i32::from(layout.streaminfo.bits) - 1);
    Ok(Peaks {
        max: f64::from(max) / full,
        min: f64::from(min) / full,
    })
}

/// The dither seed: as for WAV/AIFF, over the STREAMINFO payload (the format), the frame
/// count, the request and the first [`SEED_FRAMES`] frames of samples before the trim.
fn dither_seed(
    src: &mut File,
    path: &Path,
    layout: &FlacLayout,
    target: &Target,
    cancel: &AtomicBool,
) -> Result<u64> {
    let req = SeedRequest {
        gain_db: target.gain_db,
        trim_frames: target.trim_frames,
        bits: target.bits_out,
    };
    let mut h = seed_hasher(&layout.streaminfo_bytes, target.frames_in, req);
    let channels = usize::from(layout.streaminfo.channels);
    let mut left = usize::try_from(SEED_FRAMES.min(target.frames_in)).unwrap_or(0) * channels;
    let mut pcm = FlacPcm::open(src, path, layout, false)?;
    let mut block = Vec::new();
    while left > 0 {
        check_cancel(cancel)?;
        if pcm.next_block(&mut block)? == 0 {
            break;
        }
        let take = block.len().min(left);
        for x in &block[..take] {
            h.update(&x.to_le_bytes());
        }
        left -= take;
    }
    Ok(seed_of(&h))
}

#[cfg(test)]
mod tests;
