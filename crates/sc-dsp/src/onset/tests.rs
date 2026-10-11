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
    assert_eq!(det.detect(&vec![0.0; 22_050]), Vec::new());
}

#[test]
fn the_start_ramp_holds_the_level_it_settles_at() {
    // Steps of 17 and 14 dB, then under 3 dB: the ramp ends at hop 2.
    let env = [-108.6, -91.5, -77.1, -76.3, -74.4, -51.5, -29.8];
    assert_eq!(
        settled_start(&env),
        vec![-77.1, -77.1, -77.1, -76.3, -74.4, -51.5, -29.8]
    );
    // A start that falls (a decaying attack at the first hop) or is flat is left as it is.
    let falling = [-3.0, -3.5, -4.0, -4.4, -4.9];
    assert_eq!(settled_start(&falling), falling.to_vec());
    // A ramp that never settles is held for at most MAX_START_RAMP_MS hops.
    let climbing: Vec<f64> = (0..10).map(|i| -100.0 + 10.0 * f64::from(i)).collect();
    let held = settled_start(&climbing);
    assert!(
        held[..MAX_START_RAMP_MS]
            .iter()
            .all(|&v| v.to_bits() == climbing[MAX_START_RAMP_MS].to_bits())
    );
    assert_eq!(held[MAX_START_RAMP_MS..], climbing[MAX_START_RAMP_MS..]);
    // Inputs shorter than the ramp.
    assert_eq!(settled_start(&[]), Vec::<f64>::new());
    assert_eq!(settled_start(&[-20.0]), vec![-20.0]);
    assert_eq!(settled_start(&[-60.0, -20.0]), vec![-20.0, -20.0]);
}

#[test]
fn a_click_at_5_ms_after_silence_is_found() {
    // A 60 Hz burst 5 ms into digital silence (a cut synthetic click track): the first hops are
    // silent, so nothing ramps, and the attack is placed at its start.
    let rate = 22_050;
    let mut mono = vec![0.0_f32; rate as usize];
    let start = 110; // 5 ms
    for i in 0..882 {
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / rate as f32;
        mono[start + i] = 0.8 * (std::f32::consts::TAU * 60.0 * t).sin();
    }
    let onsets = OnsetDetector::new(rate).detect(&mono);
    assert_eq!(onsets.len(), 1, "{onsets:?}");
    assert!(onsets[0].frame.abs_diff(start) <= 22, "{onsets:?}");
}
