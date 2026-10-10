//! Unit tests of `crates/sc-dsp/src/fade.rs`: the ramp's shape (to 1e-12 against the closed
//! form with the platform cosine), its ends, monotonicity and length, and streaming across
//! block edges (bit-identical for every block size, edges inside the ramp and inside a frame).
#![allow(clippy::float_cmp)] // exact values are intended where compared exactly
use super::*;

/// The closed form with the platform's cosine, an independent reference for libm's.
fn reference(n: usize, len: usize) -> f64 {
    if n >= len {
        1.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        let x = std::f64::consts::PI * n as f64 / len as f64;
        0.5 * (1.0 - x.cos())
    }
}

/// Gains of every frame of a `len`-frame fade over `frames` frames of a constant 1.0 signal.
fn gains(len: usize, channels: u16, frames: usize) -> Vec<f64> {
    let mut f = FadeIn::new(len, channels).expect("valid");
    let mut block = vec![1.0; frames * usize::from(channels)];
    f.push(&mut block);
    block
}

#[test]
fn the_shape_is_the_rising_half_of_a_hann_window() {
    for len in [1, 2, 88, 96, 1000] {
        for n in 0..len + 5 {
            let w = raised_cosine_in(n, len);
            assert!(
                (w - reference(n, len)).abs() <= 1e-12,
                "len {len}, frame {n}: {w}"
            );
        }
    }
    // Midpoint of an even ramp: cos(pi/2) is 6e-17, so the gain is 0.5 within 1e-12.
    assert!((raised_cosine_in(48, 96) - 0.5).abs() <= 1e-12);
}

#[test]
fn starts_at_zero_rises_strictly_and_ends_at_one() {
    for (len, channels) in [(88, 2), (96, 1), (96, 2), (3, 2)] {
        let g = gains(len, channels, len + 10);
        let ch = usize::from(channels);
        let frames: Vec<f64> = g.chunks(ch).map(|f| f[0]).collect();
        for f in g.chunks(ch) {
            assert!(f.iter().all(|x| *x == f[0]), "same gain on every channel");
        }
        assert_eq!(frames[0], 0.0, "the first frame is silent");
        for w in frames[..=len].windows(2) {
            assert!(w[1] > w[0], "strictly rising: {w:?}");
        }
        assert!(frames[len - 1] < 1.0);
        assert!(
            frames[len..].iter().all(|x| *x == 1.0),
            "unity from frame N on"
        );
    }
}

#[test]
fn the_fade_is_exactly_n_frames_long() {
    let g = gains(88, 2, 200);
    assert_eq!(g.iter().filter(|x| **x < 1.0).count(), 88 * 2);
    let f = FadeIn::new(88, 2).expect("valid");
    assert_eq!((f.len_frames(), f.remaining_samples()), (88, 176));
    // A signal shorter than the fade is faded all through.
    let short = gains(96, 1, 10);
    assert_eq!(short.len(), 10);
    assert!(short.iter().all(|x| *x < 1.0));
}

#[test]
fn block_edges_inside_the_ramp_and_inside_a_frame_change_nothing() {
    let signal: Vec<f64> = (0..400)
        .map(|i| (f64::from(i) * 0.37).sin() * 0.8)
        .collect();
    let mut whole = signal.clone();
    FadeIn::new(96, 2).expect("valid").push(&mut whole);
    for size in [1, 3, 7, 64, 191, 192, 193, 65_536] {
        let mut f = FadeIn::new(96, 2).expect("valid");
        let mut streamed = signal.clone();
        for block in streamed.chunks_mut(size) {
            f.push(block);
        }
        assert_eq!(streamed, whole, "block size {size}");
        assert!(f.is_done());
    }
    // Sample by sample through `next_gain` gives the same gains.
    let mut f = FadeIn::new(96, 2).expect("valid");
    let by_gain: Vec<f64> = signal.iter().map(|x| x * f.next_gain()).collect();
    assert_eq!(by_gain, whole);
    assert_eq!(f.next_gain(), 1.0, "unity after the ramp");
}

#[test]
fn reset_starts_again_and_zero_length_is_a_no_op() {
    let mut f = FadeIn::new(4, 1).expect("valid");
    let mut a = [1.0; 6];
    f.push(&mut a);
    f.reset();
    let mut b = [1.0; 6];
    f.push(&mut b);
    assert_eq!(a, b);
    let mut none = FadeIn::new(0, 2).expect("valid");
    assert!(none.is_done());
    let mut c = [0.25; 8];
    none.push(&mut c);
    assert_eq!(c, [0.25; 8]);
    assert!(FadeIn::new(8, 0).is_err(), "no channels");
}
