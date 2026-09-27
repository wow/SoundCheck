//! Unit tests of the private parts of `crates/sc-core/src/audio.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;

#[test]
fn frames_duration_and_channels() {
    let buf = AudioBuffer::new(AudioSpec::new(4, 2), vec![0.1, -0.5, 0.2, 0.3]);
    assert_eq!(buf.frames(), 2);
    assert_eq!(buf.duration(), Seconds(0.5));
    assert_eq!(buf.channel(1).collect::<Vec<_>>(), vec![-0.5, 0.3]);
    assert_eq!(buf.peak_abs(), 0.5);
}

#[test]
fn silence_has_the_requested_length() {
    let buf = AudioBuffer::silence(AudioSpec::CD, 10);
    assert_eq!(buf.frames(), 10);
    assert_eq!(buf.data.len(), 20);
    assert_eq!(buf.peak_abs(), 0.0);
}

#[test]
#[should_panic(expected = "multiple of the channel count")]
fn rejects_ragged_interleaving() {
    let _ = AudioBuffer::new(AudioSpec::CD, vec![0.0; 3]);
}
