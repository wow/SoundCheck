//! Gain and word-length conversion to integer PCM: the last stage before a lossless writer.
//!
//! Integer input holds values at its own depth (a 24-bit sample in -2^23..2^23), float input
//! holds full scale as -1.0..1.0. The output is a two's complement integer at `out_bits`.
//!
//! - **Exact**: an integer source at 0 dB written at the same or a greater depth is shifted
//!   left by the depth difference, so 0 dB is bit-identical and 16 -> 24 is `x << 8`.
//! - **Otherwise** every sample is `r = x * g * 2^(out_bits - 1)` with `x` normalised to full
//!   scale and `g = 10^(gain_db / 20)` ([`crate::gain::db_to_linear`], a pure-Rust `pow`, so
//!   the same bits everywhere), computed in `f64` (the power-of-two scaling is exact, so
//!   `r` equals the product of the normalised value and the gain), then rounded to the nearest
//!   integer with ties to even (no bias).
//! - **Dither**: only when the result is not exact and the output has 16 bits or fewer, TPDF
//!   ([`crate::dither::Tpdf`]) is added once, before rounding. Never at 24 bits: the rounding
//!   error there lies 140 dB below full scale.
//! - **Saturation**: a value that rounds (or is dithered) past the largest or smallest code is
//!   set to that code and counted. Callers refuse inputs whose peak after gain reaches full
//!   scale, so saturation only ever moves the top code by less than one step; it is not
//!   clipping, and the count is reported.
//!
//! References: AES17-2020 (full scale, word length); Lipshitz, Wannamaker and Vanderkooy,
//! JAES 40(5), 1992 (TPDF dither before word-length reduction).

use sc_core::{Error, Result};

use crate::dither::Tpdf;
use crate::gain::db_to_linear;

/// What the input samples are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDepth {
    /// Two's complement integers holding `bits` significant bits (1..=32).
    Int {
        /// Significant bits per sample.
        bits: u16,
    },
    /// Floating point, full scale -1.0..1.0.
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// Exact left shift by this many bits.
    Shift(u32),
    /// Multiply by this factor (gain and depth scaling), then round.
    Scale(f64),
}

/// Converts blocks of samples to integers at the output depth. Allocation-free per block.
#[derive(Debug, Clone)]
pub struct Requantiser {
    source: SourceDepth,
    mode: Mode,
    dither: Option<Tpdf>,
    min: f64,
    max: f64,
    saturated: u64,
}

impl Requantiser {
    /// A requantiser from `source` to `out_bits` (8..=24) with `gain_db` (finite) applied as
    /// [`db_to_linear`]`(gain_db)` (exactly 1 at 0 dB); `dither_seed` seeds the TPDF dither when
    /// the conversion needs it.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for a gain that is not finite, otherwise as for
    /// [`Self::with_factor`].
    pub fn new(source: SourceDepth, out_bits: u16, gain_db: f64, dither_seed: u64) -> Result<Self> {
        if !gain_db.is_finite() {
            return Err(Error::InvalidArgument(format!(
                "gain {gain_db} dB is not finite"
            )));
        }
        let gain = if gain_db == 0.0 {
            1.0
        } else {
            db_to_linear(gain_db)
        };
        Self::with_factor(source, out_bits, gain, dither_seed)
    }

    /// A requantiser applying the linear `gain` factor (finite, >= 0); a factor of exactly 1
    /// on an integer source with no loss of depth is the exact shift.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for an output depth outside 8..=24, an integer source depth
    /// outside 1..=32, or a factor that is negative or not finite.
    pub fn with_factor(
        source: SourceDepth,
        out_bits: u16,
        gain: f64,
        dither_seed: u64,
    ) -> Result<Self> {
        if !(8..=24).contains(&out_bits) {
            return Err(Error::InvalidArgument(format!(
                "output depth {out_bits} bits outside 8..=24"
            )));
        }
        if !gain.is_finite() || gain < 0.0 {
            return Err(Error::InvalidArgument(format!(
                "gain factor {gain} is negative or not finite"
            )));
        }
        let out = i32::from(out_bits);
        let mode = match source {
            SourceDepth::Int { bits } if !(1..=32).contains(&bits) => {
                return Err(Error::InvalidArgument(format!(
                    "integer source depth {bits} bits outside 1..=32"
                )));
            }
            // Exactly 1 (0 dB) is the identity; any other factor needs rounding.
            SourceDepth::Int { bits }
                if gain.to_bits() == 1.0_f64.to_bits() && bits <= out_bits =>
            {
                Mode::Shift(u32::from(out_bits - bits))
            }
            SourceDepth::Int { bits } => Mode::Scale(gain * 2_f64.powi(out - i32::from(bits))),
            SourceDepth::Float => Mode::Scale(gain * 2_f64.powi(out - 1)),
        };
        let dither =
            (matches!(mode, Mode::Scale(_)) && out_bits <= 16).then(|| Tpdf::new(dither_seed));
        let full = 2_f64.powi(out - 1);
        Ok(Self {
            source,
            mode,
            dither,
            min: -full,
            max: full - 1.0,
            saturated: 0,
        })
    }

    /// Whether the conversion is an exact shift (0 dB, integer source, no loss of depth).
    #[must_use]
    pub fn is_exact(&self) -> bool {
        matches!(self.mode, Mode::Shift(_))
    }

    /// Whether TPDF dither is added.
    #[must_use]
    pub fn is_dithered(&self) -> bool {
        self.dither.is_some()
    }

    /// Samples set to the largest or smallest code so far.
    #[must_use]
    pub fn samples_saturated(&self) -> u64 {
        self.saturated
    }

    /// Converts integer samples; `out` receives `min(input.len(), out.len())` values.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when the requantiser was made for a float source.
    pub fn push_int(&mut self, input: &[i32], out: &mut [i32]) -> Result<()> {
        if self.source == SourceDepth::Float {
            return Err(Error::InvalidArgument(
                "integer samples pushed into a float requantiser".into(),
            ));
        }
        match self.mode {
            Mode::Shift(s) => {
                for (o, x) in out.iter_mut().zip(input) {
                    *o = x << s;
                }
            }
            Mode::Scale(k) => {
                for (o, x) in out.iter_mut().zip(input) {
                    *o = self.quantise(f64::from(*x) * k);
                }
            }
        }
        Ok(())
    }

    /// Converts float samples; `out` receives `min(input.len(), out.len())` values.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when the requantiser was made for an integer source.
    pub fn push_float(&mut self, input: &[f64], out: &mut [i32]) -> Result<()> {
        let Mode::Scale(k) = self.mode else {
            return Err(Error::InvalidArgument(
                "float samples pushed into an integer requantiser".into(),
            ));
        };
        if self.source != SourceDepth::Float {
            return Err(Error::InvalidArgument(
                "float samples pushed into an integer requantiser".into(),
            ));
        }
        for (o, x) in out.iter_mut().zip(input) {
            *o = self.quantise(x * k);
        }
        Ok(())
    }

    /// Dither (when on), round half to even, saturate at the output range.
    #[inline]
    fn quantise(&mut self, r: f64) -> i32 {
        let d = self.dither.as_mut().map_or(0.0, Tpdf::next_lsb);
        let mut y = (r + d).round_ties_even();
        if y > self.max {
            y = self.max;
            self.saturated += 1;
        } else if y < self.min {
            y = self.min;
            self.saturated += 1;
        } else if y.is_nan() {
            y = 0.0;
            self.saturated += 1;
        }
        // y is an integer inside the output range (at most 24 bits), so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        let q = y as i32;
        q
    }
}

#[cfg(test)]
mod tests;
