//! Raised-cosine fade-in: the gain ramp that starts audio cut out of the middle of a signal
//! from silence instead of a step.
//!
//! An `N`-frame fade multiplies frame `n` by
//!
//! ```text
//! w(n) = 0.5 - 0.5 * cos(pi * n / N)    for 0 <= n < N
//! w(n) = 1                              for n >= N
//! ```
//!
//! the rising half of a Hann window of length `2N` (F. J. Harris, "On the use of windows for
//! harmonic analysis with the discrete Fourier transform", Proc. IEEE 66(1), 1978): `w(0)` is
//! exactly 0, the ramp rises strictly and reaches exactly 1 at frame `N`, and its slope is zero
//! at both ends, so neither end adds a click. Every channel of a frame gets the same gain. The
//! cosine is the pure-Rust `libm` port of musl's, so the gains are bit-identical on every
//! platform.
//!
//! [`FadeIn`] is streaming: blocks of any size (a block edge may fall inside the ramp, or inside
//! a frame) continue where the previous block stopped, and pushing allocates nothing.

use sc_core::{Error, Result};

/// Gain of frame `n` of a raised-cosine fade-in `len` frames long (`len` > 0): 0 at `n = 0`,
/// rising to 1 at `n = len` and staying 1 after it.
#[must_use]
pub fn raised_cosine_in(n: usize, len: usize) -> f64 {
    if n >= len {
        return 1.0;
    }
    // Fade lengths and positions are a few hundred frames: exact in f64.
    #[allow(clippy::cast_precision_loss)]
    let phase = std::f64::consts::PI * n as f64 / len as f64;
    0.5 - 0.5 * libm::cos(phase)
}

/// A streaming raised-cosine fade-in over interleaved samples (see the module documentation).
#[derive(Debug, Clone, PartialEq)]
pub struct FadeIn {
    /// `w(0)..w(N-1)`, computed once.
    ramp: Vec<f64>,
    /// Interleaved channels.
    channels: usize,
    /// Samples (not frames) faded so far, at most `ramp.len() * channels`.
    done: usize,
}

impl FadeIn {
    /// A fade over the first `len_frames` frames of `channels` interleaved channels. A length of
    /// 0 is a fade that is already over (every gain 1).
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for 0 channels.
    pub fn new(len_frames: usize, channels: u16) -> Result<Self> {
        if channels == 0 {
            return Err(Error::InvalidArgument("a fade over 0 channels".into()));
        }
        Ok(Self {
            ramp: (0..len_frames)
                .map(|n| raised_cosine_in(n, len_frames))
                .collect(),
            channels: usize::from(channels),
            done: 0,
        })
    }

    /// Length of the ramp, frames.
    #[must_use]
    pub fn len_frames(&self) -> usize {
        self.ramp.len()
    }

    /// Interleaved samples still inside the ramp.
    #[must_use]
    pub fn remaining_samples(&self) -> usize {
        self.ramp.len() * self.channels - self.done
    }

    /// Whether the ramp is over: every further gain is 1.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.remaining_samples() == 0
    }

    /// The gain of the next interleaved sample, moving on by one sample; 1 once the ramp is
    /// over.
    #[inline]
    pub fn next_gain(&mut self) -> f64 {
        match self.ramp.get(self.done / self.channels) {
            Some(w) => {
                self.done += 1;
                *w
            }
            None => 1.0,
        }
    }

    /// Multiplies the next samples of the stream, `block` (interleaved), by their gains in
    /// place; samples after the ramp are left as they are. Allocation-free.
    pub fn push(&mut self, block: &mut [f64]) {
        let n = self.remaining_samples().min(block.len());
        for x in &mut block[..n] {
            *x *= self.next_gain();
        }
    }

    /// Starts the ramp again from frame 0.
    pub fn reset(&mut self) {
        self.done = 0;
    }
}

#[cfg(test)]
mod tests;
