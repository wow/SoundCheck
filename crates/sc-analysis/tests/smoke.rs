//! Sample peak on a synthetic sine.

use approx::assert_abs_diff_eq;
use sc_core::{AudioSpec, testsig};

#[test]
fn sine_at_half_scale_peaks_at_minus_six() {
    let buf = testsig::sine(AudioSpec::CD, 440.0, 0.5, 0.5);
    assert_abs_diff_eq!(
        sc_analysis::sample_peak(&buf.data).0,
        -6.020_6,
        epsilon = 1e-3
    );
}
