//! Unit tests of the private parts of `crates/sc-dsp/src/onset.rs`.
use super::*;

#[test]
fn median_of_odd_and_even_lengths() {
    assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-12);
    assert!((median(&[4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < 1e-12);
}

#[test]
fn silence_has_no_onsets() {
    let det = OnsetDetector::new(22_050);
    assert_eq!(det.hop_frames(), 22);
    assert!(det.detect(&vec![0.0; 22_050]).is_empty());
}
