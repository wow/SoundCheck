//! Gain in decibels and linear amplitude, plus a streaming gain stage.

use sc_core::units::{DbFs, MIN_DBFS};

/// Linear amplitude factor for a gain in dB: `10^(db / 20)`.
///
/// Computed with the pure-Rust `libm` port of musl's `pow`, not the platform's maths library,
/// so a rendered file is bit-identical on every operating system and architecture.
#[must_use]
pub fn db_to_linear(db: f64) -> f64 {
    libm::pow(10.0, db / 20.0)
}

/// Gain in dB for a linear amplitude factor; non-positive input clamps to [`MIN_DBFS`].
#[must_use]
pub fn linear_to_db(linear: f64) -> f64 {
    if linear <= 0.0 {
        MIN_DBFS
    } else {
        (20.0 * linear.log10()).max(MIN_DBFS)
    }
}

/// Multiplies every sample in place. Allocation-free.
pub fn apply_gain(block: &mut [f32], linear: f32) {
    for s in block {
        *s *= linear;
    }
}

/// A streaming gain stage: the same factor applied to every block pushed through it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gain {
    linear: f32,
}

impl Gain {
    /// A stage applying `gain`.
    #[must_use]
    // A gain factor is a small number; narrowing to f32 is the intended precision.
    #[allow(clippy::cast_possible_truncation)]
    pub fn new(gain: DbFs) -> Self {
        Self {
            linear: gain.to_linear() as f32,
        }
    }

    /// The linear factor applied.
    #[must_use]
    pub fn linear(self) -> f32 {
        self.linear
    }

    /// Applies the gain to one block in place.
    pub fn push(&mut self, block: &mut [f32]) {
        apply_gain(block, self.linear);
    }
}

#[cfg(test)]
mod tests;
