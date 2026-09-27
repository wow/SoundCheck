//! Unit tests of the private parts of `crates/sc-analysis/src/peak.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;
use approx::assert_abs_diff_eq;
use sc_core::units::MIN_DBFS;

#[test]
fn half_scale_is_minus_six_db() {
    assert_abs_diff_eq!(sample_peak(&[0.1, -0.5, 0.2]).0, -6.020_6, epsilon = 1e-4);
    assert_eq!(sample_peak(&[0.0; 8]), DbFs(MIN_DBFS));
    assert_eq!(sample_peak(&[]), DbFs(MIN_DBFS));
}
