//! TPDF (triangular probability density) dither for word-length reduction.
//!
//! Each value is the sum of two independent uniform values over [-0.5, 0.5) LSB, so it is
//! triangular over (-1, 1) LSB with mean 0 and variance 1/6 LSB^2. Added before rounding, it
//! makes the first and second moments of the total error independent of the signal (Lipshitz,
//! Wannamaker and Vanderkooy, "Quantization and Dither: A Theoretical Survey", JAES 40(5),
//! 1992; Wannamaker, Lipshitz and Vanderkooy, "A Theory of Nonsubtractive Dither", IEEE Trans.
//! Signal Processing 48(2), 2000); the total error then has an RMS of 1/2 LSB (1/12 from
//! rounding plus 1/6 from the dither).
//!
//! The random source is xorshift64* (Vigna, "An experimental exploration of Marsaglia's
//! xorshift generators, scrambled", ACM TOMS 42(4), 2016), seeded by the caller and never from
//! the clock: the same seed gives the same sequence on every platform. Each uniform value uses
//! the top 53 bits of one output, so it is an exact multiple of 2^-53.

/// A seeded TPDF dither source; one value per sample, in LSB of the output word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tpdf {
    state: u64,
}

/// State used in place of a zero seed (xorshift has a fixed point at 0).
const ZERO_SEED_STATE: u64 = 0x9E37_79B9_7F4A_7C15;

/// 2^-53: one step of a 53-bit uniform value.
const UNIT_53: f64 = f64::EPSILON / 2.0;

impl Tpdf {
    /// A dither source for `seed` (any value; 0 is mapped to a fixed non-zero state).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { ZERO_SEED_STATE } else { seed },
        }
    }

    /// Next 64 random bits (xorshift64*).
    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform over [-0.5, 0.5) LSB.
    #[inline]
    fn next_uniform_lsb(&mut self) -> f64 {
        // The top 53 bits fit an f64 mantissa exactly.
        #[allow(clippy::cast_precision_loss)]
        let unit = (self.next_u64() >> 11) as f64 * UNIT_53;
        unit - 0.5
    }

    /// The next dither value, LSB: triangular over (-1, 1), mean 0. Allocation-free.
    #[inline]
    #[must_use]
    pub fn next_lsb(&mut self) -> f64 {
        self.next_uniform_lsb() + self.next_uniform_lsb()
    }

    /// Fills `out` with consecutive dither values, LSB. Allocation-free.
    pub fn fill(&mut self, out: &mut [f64]) {
        for v in out {
            *v = self.next_lsb();
        }
    }
}

#[cfg(test)]
mod tests;
