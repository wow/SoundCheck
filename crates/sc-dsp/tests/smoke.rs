//! Gain on a synthetic signal.

use approx::assert_abs_diff_eq;
use sc_core::{AudioSpec, DbFs, testsig};
use sc_dsp::Gain;

#[test]
fn minus_six_db_halves_a_sine() {
    let mut buf = testsig::sine(AudioSpec::CD, 440.0, 1.0, 0.1);
    Gain::new(DbFs(-6.020_6)).push(&mut buf.data);
    assert_abs_diff_eq!(buf.peak_abs(), 0.5, epsilon = 1e-4);
}
