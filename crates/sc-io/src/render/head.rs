//! The head cut's two refinements: where exactly the cut falls, and the fade after it.
//!
//! - **Snap back**: a requested cut of `T` frames (`T` > 0) becomes `T'`, the frame in
//!   `[T - w, T]` (clamped at 0; `w` = [`head_snap_frames`], 1 ms rounded to whole frames: 44
//!   at 44.1 kHz, 48 at 48 kHz) whose largest absolute sample across channels is smallest,
//!   the latest such frame on a tie. The output starts with frame `T'`. The cut never moves
//!   later than requested: a caller places `T` so that a bar line lands at a lead after the
//!   start, and a later cut would put it before the lead. Samples are compared as stored (the
//!   gain is the same for every frame, so it cannot change the choice); a NaN float sample
//!   counts as the loudest.
//! - **Fade-in**: after a cut (`T'` > 0) the first [`head_fade_frames`] output frames (2 ms
//!   rounded: 88 at 44.1 kHz, 96 at 48 kHz) are multiplied by a raised-cosine ramp from 0 to 1
//!   (`sc_dsp::fade`) in floating point before rounding and dither, identically on every
//!   channel. Nothing is faded when nothing is cut. The fade only lowers samples, so the
//!   full-scale check before a boost, made on the samples from `T'` on without it, stays valid
//!   (and is at worst cautious about a peak inside the first 2 ms).
//!
//! The snapped frame need not be a zero crossing (noise or a sustained low note may have none
//! within 1 ms); the fade makes any starting value safe, and the snap keeps the step the fade
//! smooths as small as the 1 ms allows.

/// Largest distance, ms, by which a head cut moves earlier to a quieter frame.
pub const HEAD_SNAP_MAX_MS: u32 = 1;

/// Length, ms, of the fade-in after a head cut.
pub const HEAD_FADE_MS: u32 = 2;

/// `ms` milliseconds in frames at `sample_rate_hz`, rounded half up.
fn ms_frames(sample_rate_hz: u32, ms: u32) -> u64 {
    (u64::from(sample_rate_hz) * u64::from(ms) + 500) / 1000
}

/// Largest distance, frames, by which a head cut moves earlier: [`HEAD_SNAP_MAX_MS`] at
/// `sample_rate_hz`, rounded (44 at 44,100 Hz, 48 at 48,000 Hz).
#[must_use]
pub fn head_snap_frames(sample_rate_hz: u32) -> u64 {
    ms_frames(sample_rate_hz, HEAD_SNAP_MAX_MS)
}

/// Length, frames, of the fade-in after a head cut: [`HEAD_FADE_MS`] at `sample_rate_hz`,
/// rounded (88 at 44,100 Hz, 96 at 48,000 Hz).
#[must_use]
pub fn head_fade_frames(sample_rate_hz: u32) -> usize {
    usize::try_from(ms_frames(sample_rate_hz, HEAD_FADE_MS)).unwrap_or(usize::MAX)
}

/// Chooses the snapped cut from the first frames of the source, pushed in blocks of whole
/// frames from frame 0 on.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HeadSnap {
    /// First frame of the window.
    first: u64,
    /// Last frame of the window: the requested cut.
    last: u64,
    channels: usize,
    /// Index of the next frame pushed.
    next: u64,
    /// The quietest frame so far and its largest absolute sample.
    best: Option<(u64, f64)>,
}

impl HeadSnap {
    /// A snap for the requested cut of `requested` frames (> 0) of a source at
    /// `sample_rate_hz` with `channels` interleaved channels (> 0).
    pub(super) fn new(requested: u64, sample_rate_hz: u32, channels: u16) -> Self {
        Self {
            first: requested.saturating_sub(head_snap_frames(sample_rate_hz)),
            last: requested,
            channels: usize::from(channels.max(1)),
            next: 0,
            best: None,
        }
    }

    /// Whether frames of the window are still to come.
    pub(super) fn wants_more(&self) -> bool {
        self.next <= self.last
    }

    /// Takes the next block of whole frames; `magnitude` gives a sample's absolute value.
    pub(super) fn push<T: Copy>(&mut self, block: &[T], magnitude: fn(T) -> f64) {
        let frames = (block.len() / self.channels) as u64;
        let skip = self.first.saturating_sub(self.next).min(frames);
        let take = (self.last + 1)
            .saturating_sub(self.next + skip)
            .min(frames - skip);
        let channels = self.channels;
        // Both bounds are at most the block's frame count, which fits a usize.
        let (skip_us, take_us) = (
            usize::try_from(skip).unwrap_or(0),
            usize::try_from(take).unwrap_or(0),
        );
        let window = &block[skip_us * channels..(skip_us + take_us) * channels];
        for (j, frame) in window.chunks_exact(channels).enumerate() {
            let m = frame.iter().map(|x| magnitude(*x)).fold(0.0_f64, f64::max);
            if self.best.is_none_or(|(_, b)| m <= b) {
                self.best = Some((self.next + skip + j as u64, m));
            }
        }
        self.next += frames;
    }

    /// The snapped cut: the quietest frame of the window (the latest on a tie), or the
    /// requested cut when the window was never reached.
    pub(super) fn finish(&self) -> u64 {
        self.best.map_or(self.last, |(frame, _)| frame)
    }
}

/// A stored integer sample's absolute value.
pub(super) fn int_magnitude(x: i32) -> f64 {
    f64::from(x).abs()
}

/// A float sample's absolute value; NaN counts as the loudest.
pub(super) fn float_magnitude(x: f64) -> f64 {
    if x.is_nan() { f64::INFINITY } else { x.abs() }
}

#[cfg(test)]
mod tests;
