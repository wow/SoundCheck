//! Unit tests of the private parts of `crates/sc-dsp/src/gain.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;
use approx::assert_abs_diff_eq;

#[test]
fn db_and_linear_agree() {
    assert_abs_diff_eq!(db_to_linear(0.0), 1.0);
    assert_abs_diff_eq!(db_to_linear(-6.020_6), 0.5, epsilon = 1e-5);
    assert_abs_diff_eq!(linear_to_db(2.0), 6.020_6, epsilon = 1e-4);
    assert_eq!(linear_to_db(0.0), MIN_DBFS);
}

#[test]
fn gain_scales_samples() {
    let mut block = [1.0_f32, -0.5, 0.25];
    Gain::new(DbFs(-6.020_6)).push(&mut block);
    assert_abs_diff_eq!(block[0], 0.5, epsilon = 1e-5);
    assert_abs_diff_eq!(block[1], -0.25, epsilon = 1e-5);
}
