//! Unit tests of the private parts of `crates/sc-dsp/src/downmix.rs`.
use super::*;

#[test]
fn stereo_averages_and_mono_copies() {
    let mut out = Vec::new();
    to_mono(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2, &mut out);
    assert_eq!(out, vec![0.5, 0.5, 0.0]);
    to_mono(&[0.25, -0.25], 1, &mut out);
    assert_eq!(out, vec![0.5, 0.5, 0.0, 0.25, -0.25]);
}
