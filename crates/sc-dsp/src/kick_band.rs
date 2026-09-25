//! The kick band: 30-150 Hz, a second-order Butterworth high-pass followed by a second-order
//! Butterworth low-pass. Kick drums put their energy here, and the band's energy rise marks the
//! beat's onset far more sharply than a broadband envelope does.

use std::f64::consts::FRAC_1_SQRT_2;

use crate::biquad::Biquad;

/// Lower edge of the band, in Hz.
pub const KICK_LOW_HZ: f64 = 30.0;
/// Upper edge of the band, in Hz.
pub const KICK_HIGH_HZ: f64 = 150.0;

/// A mono band-pass for the kick band.
#[derive(Debug, Clone)]
pub struct KickBand {
    highpass: Biquad,
    lowpass: Biquad,
}

impl KickBand {
    /// A filter for `sample_rate` (any rate above 300 Hz).
    ///
    /// # Panics
    /// When `sample_rate` is too low for the band edges.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            highpass: Biquad::highpass(sample_rate, KICK_LOW_HZ, FRAC_1_SQRT_2),
            lowpass: Biquad::lowpass(sample_rate, KICK_HIGH_HZ, FRAC_1_SQRT_2),
        }
    }

    /// Clears the filter state.
    pub fn reset(&mut self) {
        self.highpass.reset();
        self.lowpass.reset();
    }

    /// Filters one sample.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.lowpass.process(self.highpass.process(x))
    }

    /// Filters a mono block in place.
    pub fn process_block(&mut self, block: &mut [f32]) {
        for x in block {
            *x = self.process(*x);
        }
    }

    /// Band magnitude at `freq_hz`, in dB.
    #[must_use]
    pub fn magnitude_db(&self, sample_rate: u32, freq_hz: f64) -> f64 {
        self.highpass.magnitude_db(sample_rate, freq_hz)
            + self.lowpass.magnitude_db(sample_rate, freq_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn passes_the_band_and_rejects_the_rest() {
        let band = KickBand::new(22_050);
        assert_abs_diff_eq!(band.magnitude_db(22_050, 70.0), 0.0, epsilon = 0.6);
        assert_abs_diff_eq!(band.magnitude_db(22_050, KICK_LOW_HZ), -3.0, epsilon = 0.3);
        assert_abs_diff_eq!(band.magnitude_db(22_050, KICK_HIGH_HZ), -3.0, epsilon = 0.3);
        assert!(band.magnitude_db(22_050, 1_000.0) < -30.0);
        assert!(band.magnitude_db(22_050, 5.0) < -25.0);
    }
}
