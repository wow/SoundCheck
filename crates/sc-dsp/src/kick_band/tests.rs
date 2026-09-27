//! Unit tests of the private parts of `crates/sc-dsp/src/kick_band.rs`.
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
