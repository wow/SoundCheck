//! Unit tests of the private parts of `crates/sc-core/src/testsig.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;
use approx::assert_abs_diff_eq;

#[test]
fn sine_has_the_requested_amplitude_and_length() {
    let buf = sine(AudioSpec::CD, 1000.0, 0.5, 1.0);
    assert_eq!(buf.frames(), 44_100);
    assert_abs_diff_eq!(buf.peak_abs(), 0.5, epsilon = 1e-4);
}

#[test]
fn click_track_places_beats_on_the_expected_frames() {
    let spec = AudioSpec::new(48_000, 1);
    let buf = click_track(spec, Bpm(120.0), 8, 5.0);
    // Beat 2 starts at 0.5 s = frame 24 000; the sample before it is silent.
    assert_eq!(buf.data[23_999], 0.0);
    assert_ne!(buf.data[24_001], 0.0);
    // Downbeats are louder than other beats.
    let bar_peak = buf.data[0..240].iter().fold(0.0_f32, |a, s| a.max(s.abs()));
    let beat_peak = buf.data[24_000..24_240]
        .iter()
        .fold(0.0_f32, |a, s| a.max(s.abs()));
    assert!(bar_peak > beat_peak);
}

#[test]
fn noise_is_deterministic_and_bounded() {
    let a = seeded_noise(AudioSpec::CD, 7, 0.25, 0.1);
    let b = seeded_noise(AudioSpec::CD, 7, 0.25, 0.1);
    assert_eq!(a, b);
    assert!(a.peak_abs() <= 0.25);
    assert_ne!(a, seeded_noise(AudioSpec::CD, 8, 0.25, 0.1));
}

#[test]
fn impulse_is_a_single_sample() {
    let buf = impulse(AudioSpec::new(1000, 2), SampleIndex(10), 0.1);
    assert_eq!(buf.data.iter().filter(|s| **s != 0.0).count(), 2);
    assert_eq!(buf.data[20], 1.0);
}
