//! Unit tests of the private parts of `crates/sc-dsp/src/biquad.rs`.
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
