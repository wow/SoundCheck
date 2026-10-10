//! Unit tests of `crates/sc-io/src/render/head.rs`: the window, the quietest frame with ties
//! to the latest, never later than requested, clamped at 0, and the same choice for every
//! block size.
use super::*;

/// The snapped cut of `requested` over interleaved `samples`, pushed in blocks of `frames`.
fn snap(samples: &[i32], channels: u16, requested: u64, frames: usize) -> u64 {
    let mut s = HeadSnap::new(requested, 44_100, channels);
    for block in samples.chunks(frames * usize::from(channels)) {
        if !s.wants_more() {
            break;
        }
        s.push(block, int_magnitude);
    }
    s.finish()
}

#[test]
fn lengths_round_to_whole_frames() {
    assert_eq!(
        (head_snap_frames(44_100), head_snap_frames(48_000)),
        (44, 48)
    );
    assert_eq!(
        (head_fade_frames(44_100), head_fade_frames(48_000)),
        (88, 96)
    );
}

#[test]
fn the_quietest_frame_of_the_window_wins_and_ties_go_late() {
    // Loud everywhere, quiet at 450 (outside, later), 380 (outside, earlier), 420 and 430.
    let mut mono = vec![1000_i32; 600];
    mono[450] = 0;
    mono[380] = 0;
    mono[420] = 5;
    mono[430] = -5;
    assert_eq!(
        snap(&mono, 1, 441, 4096),
        430,
        "tie at |5|: the later frame"
    );
    mono[425] = 1;
    assert_eq!(snap(&mono, 1, 441, 4096), 425);
    // The window is [397, 441]: both ends are in it.
    mono[397] = 0;
    assert_eq!(snap(&mono, 1, 441, 4096), 397);
    mono[441] = 0;
    assert_eq!(
        snap(&mono, 1, 441, 4096),
        441,
        "the requested frame on a tie"
    );
}

#[test]
fn no_quieter_frame_keeps_the_request() {
    let flat = vec![-700_i32; 1000];
    assert_eq!(snap(&flat, 1, 441, 4096), 441);
    assert_eq!(snap(&flat, 1, 441, 1), 441);
}

#[test]
fn stereo_compares_the_louder_channel_and_the_window_clamps_at_zero() {
    // Frame 10: left silent, right loud; frame 12: both moderate.
    let mut st = vec![900_i32; 2 * 100];
    st[20] = 0;
    st[21] = 950;
    st[24] = 300;
    st[25] = -300;
    assert_eq!(snap(&st, 2, 30, 4096), 12);
    let mut early = vec![50_i32; 100];
    early[0] = 0;
    assert_eq!(snap(&early, 1, 30, 4096), 0, "window [0, 30]");
}

#[test]
fn every_block_size_gives_the_same_cut() {
    let samples: Vec<i32> = (0..4000_i32).map(|i| ((i * 7919) % 2001) - 1000).collect();
    let want = snap(&samples, 2, 1500, 4096);
    for frames in [1, 3, 44, 45, 1000] {
        assert_eq!(snap(&samples, 2, 1500, frames), want, "{frames} frames");
    }
}

#[test]
fn nan_counts_as_the_loudest() {
    let mut s = HeadSnap::new(3, 44_100, 1);
    s.push(&[0.5, f64::NAN, 0.25, 0.75], float_magnitude);
    assert_eq!(s.finish(), 2);
}
