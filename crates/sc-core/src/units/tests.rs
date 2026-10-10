//! Unit tests of the private parts of `crates/sc-core/src/units.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;
use approx::assert_abs_diff_eq;

#[test]
fn dbfs_round_trips_through_linear() {
    assert_abs_diff_eq!(DbFs::from_linear(0.5).0, -6.020_6, epsilon = 1e-4);
    assert_abs_diff_eq!(DbFs(-6.0).to_linear(), 0.501_187, epsilon = 1e-6);
    assert_eq!(DbFs::from_linear(0.0), DbFs(MIN_DBFS));
}

#[test]
fn samples_and_seconds_convert_both_ways() {
    let idx = SampleIndex(44_100);
    assert_eq!(idx.to_seconds(44_100), Seconds(1.0));
    assert_eq!(Seconds(0.5).to_sample_index(48_000), SampleIndex(24_000));
    assert_eq!(Seconds(-1.0).to_sample_index(48_000), SampleIndex(0));
}

#[test]
fn loudness_difference_is_lu() {
    assert_eq!(Lufs(-8.0) - Lufs(-11.0), Lu(3.0));
    assert_eq!(format!("{}", Lufs(-11.0)), "-11.00 LUFS");
}

#[test]
fn units_serialise_transparently() {
    assert_eq!(serde_json::to_string(&Bpm(128.0)).unwrap(), "128.0");
    assert_eq!(serde_json::to_string(&SampleIndex(7)).unwrap(), "7");
}

#[test]
fn written_bpm_rounds_to_two_decimals() {
    assert_eq!(Bpm(119.996).written(), Bpm(120.0));
    assert_eq!(Bpm(127.984_9).written(), Bpm(127.98));
    assert_eq!(Bpm(174.125).written(), Bpm(174.13));
    // Rounding acts on the binary value: 1.005 is stored just below the half.
    assert_eq!(Bpm(1.005).written(), Bpm(1.0));
    assert_eq!(Bpm(93.455).written(), Bpm(93.46));
    assert_eq!(Bpm(128.0).written(), Bpm(128.0));
    // Already two decimals: unchanged, so writing twice is the same as writing once.
    let once = Bpm(93.456).written();
    assert_eq!(once.written(), once);
    assert_eq!(format!("{:.2}", once.0), "93.46");
}
