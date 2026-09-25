//! Second-order IIR sections from the Audio EQ Cookbook (Robert Bristow-Johnson), in transposed
//! direct form II with `f64` coefficients and state, processing `f32` samples one channel at a
//! time. Used for the kick-band filter and, later, K-weighting checks and the limiter sidechain.

use std::f64::consts::PI;

/// One biquad section.
#[derive(Debug, Clone, PartialEq)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// A section from normalised coefficients (`a0` already divided out).
    #[must_use]
    pub const fn from_coefficients(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Second-order low-pass at `f0_hz` with quality `q` (`FRAC_1_SQRT_2` for Butterworth).
    ///
    /// # Panics
    /// When `f0_hz` is not inside `0 < f0 < sample_rate / 2` or `q` is not positive.
    #[must_use]
    pub fn lowpass(sample_rate: u32, f0_hz: f64, q: f64) -> Self {
        let (cos_w0, alpha) = Self::intermediates(sample_rate, f0_hz, q);
        let a0 = 1.0 + alpha;
        Self::from_coefficients(
            f64::midpoint(1.0, -cos_w0) / a0,
            (1.0 - cos_w0) / a0,
            f64::midpoint(1.0, -cos_w0) / a0,
            -2.0 * cos_w0 / a0,
            (1.0 - alpha) / a0,
        )
    }

    /// Second-order high-pass at `f0_hz` with quality `q`.
    ///
    /// # Panics
    /// When `f0_hz` is not inside `0 < f0 < sample_rate / 2` or `q` is not positive.
    #[must_use]
    pub fn highpass(sample_rate: u32, f0_hz: f64, q: f64) -> Self {
        let (cos_w0, alpha) = Self::intermediates(sample_rate, f0_hz, q);
        let a0 = 1.0 + alpha;
        Self::from_coefficients(
            f64::midpoint(1.0, cos_w0) / a0,
            -(1.0 + cos_w0) / a0,
            f64::midpoint(1.0, cos_w0) / a0,
            -2.0 * cos_w0 / a0,
            (1.0 - alpha) / a0,
        )
    }

    fn intermediates(sample_rate: u32, f0_hz: f64, q: f64) -> (f64, f64) {
        let nyquist = f64::from(sample_rate) / 2.0;
        assert!(
            f0_hz > 0.0 && f0_hz < nyquist,
            "f0 {f0_hz} Hz must lie inside 0..{nyquist} Hz"
        );
        assert!(q > 0.0, "q must be positive");
        let w0 = 2.0 * PI * f0_hz / f64::from(sample_rate);
        (w0.cos(), w0.sin() / (2.0 * q))
    }

    /// Clears the delay state.
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Filters one sample.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let x = f64::from(x);
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        // The output is audio in [-1, 1] scale; f32 is the working precision of every buffer.
        #[allow(clippy::cast_possible_truncation)]
        let y = y as f32;
        y
    }

    /// Filters a mono block in place.
    pub fn process_block(&mut self, block: &mut [f32]) {
        for x in block {
            *x = self.process(*x);
        }
    }

    /// Magnitude response at `freq_hz`, in dB (for tests and the calibration reports).
    #[must_use]
    pub fn magnitude_db(&self, sample_rate: u32, freq_hz: f64) -> f64 {
        let w = 2.0 * PI * freq_hz / f64::from(sample_rate);
        let (c1, s1) = (w.cos(), w.sin());
        let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
        // H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2) at z = e^{jw}.
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        let num = num_re.hypot(num_im);
        let den = den_re.hypot(den_im);
        20.0 * (num / den).log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;
    use std::f64::consts::FRAC_1_SQRT_2;

    #[test]
    fn butterworth_lowpass_is_minus_3_db_at_f0_and_flat_below() {
        let lp = Biquad::lowpass(44_100, 150.0, FRAC_1_SQRT_2);
        assert_abs_diff_eq!(lp.magnitude_db(44_100, 150.0), -3.01, epsilon = 0.05);
        assert_abs_diff_eq!(lp.magnitude_db(44_100, 10.0), 0.0, epsilon = 0.01);
        assert!(lp.magnitude_db(44_100, 1_500.0) < -38.0, "-40 dB/decade");
    }

    #[test]
    fn butterworth_highpass_is_minus_3_db_at_f0_and_flat_above() {
        let hp = Biquad::highpass(48_000, 30.0, FRAC_1_SQRT_2);
        assert_abs_diff_eq!(hp.magnitude_db(48_000, 30.0), -3.01, epsilon = 0.05);
        assert_abs_diff_eq!(hp.magnitude_db(48_000, 3_000.0), 0.0, epsilon = 0.01);
        assert!(hp.magnitude_db(48_000, 3.0) < -38.0);
    }

    #[test]
    fn time_domain_gain_matches_the_response() {
        // A 1 kHz tone through a 150 Hz low-pass: the steady-state amplitude follows |H|.
        let sr = 44_100;
        let mut lp = Biquad::lowpass(sr, 150.0, FRAC_1_SQRT_2);
        let n = sr as usize;
        let mut block: Vec<f32> = (0..n)
            .map(|i| {
                // Test signal; the cast is exact for these small values.
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let t = i as f64 / f64::from(sr);
                #[allow(clippy::cast_possible_truncation)]
                let sample = ((2.0 * PI * 1000.0 * t).sin() * 0.5) as f32;
                sample
            })
            .collect();
        lp.process_block(&mut block);
        let peak = block[n / 2..].iter().fold(0.0_f32, |m, x| m.max(x.abs()));
        let expected = 0.5 * 10f64.powf(lp.magnitude_db(sr, 1000.0) / 20.0);
        assert_abs_diff_eq!(f64::from(peak), expected, epsilon = expected * 0.02);
    }

    #[test]
    fn reset_clears_the_state() {
        let mut hp = Biquad::highpass(44_100, 30.0, FRAC_1_SQRT_2);
        hp.process(1.0);
        hp.reset();
        let fresh = Biquad::highpass(44_100, 30.0, FRAC_1_SQRT_2);
        assert_eq!(hp, fresh);
    }
}
