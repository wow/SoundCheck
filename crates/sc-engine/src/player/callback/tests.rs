//! Unit tests of the private parts of `crates/sc-engine/src/player/callback.rs`: the gain ramps.
use super::*;

/// +6 dB as a linear gain.
const PLUS_6_DB: f32 = 1.995_262_3;

/// A playing callback at 48 kHz stereo whose ring holds `frames` frames of `level`.
fn playing(level: f32, frames: usize) -> (Callback, Arc<Shared>, rtrb::Producer<f32>) {
    let shared = Arc::new(Shared::default());
    let (mut producer, consumer) = rtrb::RingBuffer::new(1 << 16);
    for _ in 0..frames * 2 {
        producer.push(level).expect("the ring has room");
    }
    shared.playing.store(true, Ordering::Release);
    let callback = Callback::new(consumer, Arc::clone(&shared), 2, 48_000);
    (callback, shared, producer)
}

/// Plays `frames` frames in device buffers of 128 frames; returns the left channel.
fn left(callback: &mut Callback, frames: usize) -> Vec<f32> {
    let mut heard = Vec::new();
    let mut buffer = [0.0_f32; 256];
    while heard.len() < frames {
        callback.fill(&mut buffer);
        heard.extend(buffer.iter().step_by(2));
    }
    heard.truncate(frames);
    heard
}

#[test]
fn a_new_version_gain_is_reached_in_ten_milliseconds_along_a_straight_line() {
    let (mut callback, shared, _producer) = playing(0.25, 4_000);
    let _ = left(&mut callback, 128);
    shared.set_listen_gain(0.5);
    let heard = left(&mut callback, 1_000);
    // 10 ms at 48 kHz is 480 frames: frame k of the ramp is at 1 + (0.5 - 1) * (k + 1) / 480.
    for (k, &x) in heard.iter().enumerate().take(480) {
        #[allow(clippy::cast_precision_loss)]
        let expected = 0.25 - 0.125 * (k as f32 + 1.0) / 480.0;
        assert!((x - expected).abs() < 1e-6, "frame {k}: {x} vs {expected}");
    }
    assert_eq!(heard[479], 0.125, "the target at the 480th frame");
    assert!(
        heard[480..]
            .iter()
            .all(|&x| x.to_bits() == 0.125_f32.to_bits()),
        "and held there"
    );
}

#[test]
fn on_a_constant_input_the_ramp_is_monotone_with_no_step_above_its_increment() {
    let (mut callback, shared, _producer) = playing(0.5, 8_000);
    let _ = left(&mut callback, 128);
    shared.set_listen_gain(PLUS_6_DB);
    let up = left(&mut callback, 600);
    let increment = 0.5 * (PLUS_6_DB - 1.0) / 480.0;
    let steps: Vec<f32> = up.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(steps.iter().all(|&d| d >= 0.0), "monotone");
    assert!(
        steps.iter().all(|&d| d <= increment + 1e-6),
        "largest step {} vs increment {increment}",
        steps.iter().fold(0.0_f32, |m, &d| m.max(d))
    );
    shared.set_volume(0.0);
    let down = left(&mut callback, 600);
    assert!(down.windows(2).all(|w| w[1] <= w[0]), "monotone down");
    assert_eq!(down[479], 0.0, "muted after 10 ms");
}

#[test]
fn a_change_midway_heads_for_the_new_target_from_where_the_ramp_is() {
    let (mut callback, shared, _producer) = playing(1.0, 4_000);
    shared.set_volume(0.0);
    // Two device buffers: 256 of the 480 frames down.
    let first = left(&mut callback, 256);
    let midway = 1.0 - 256.0 / 480.0;
    assert!((first[255] - midway).abs() < 1e-6, "{}", first[255]);
    shared.set_volume(1.0);
    let back = left(&mut callback, 480);
    assert!(back.windows(2).all(|w| w[1] >= w[0]), "no jump back up");
    let step = (1.0 - midway) / 480.0;
    assert!((back[0] - (midway + step)).abs() < 1e-6, "{}", back[0]);
    assert_eq!(back[479], 1.0);
}

#[test]
fn the_clamp_comes_before_the_volume_so_turning_down_shows_the_over() {
    let (mut callback, shared, _producer) = playing(0.8, 4_000);
    shared.set_listen_gain(2.0);
    shared.set_volume(0.5);
    // Nothing plays while the gains change, so they apply at once from the next buffer.
    shared.playing.store(false, Ordering::Release);
    let _ = left(&mut callback, 128);
    shared.playing.store(true, Ordering::Release);
    let heard = left(&mut callback, 256);
    // 0.8 x 2 = 1.6 clips to 1.0, then the volume halves it: a flat 0.5, not 0.8.
    assert!(
        heard.iter().all(|&x| x.to_bits() == 0.5_f32.to_bits()),
        "{:?}",
        &heard[..4]
    );
}

#[test]
fn ramps_settle_while_paused_and_start_at_the_shared_gains() {
    let shared = Arc::new(Shared::default());
    shared.set_listen_gain(0.25);
    let (_producer, consumer) = rtrb::RingBuffer::<f32>::new(16);
    let callback = Callback::new(consumer, Arc::clone(&shared), 2, 44_100);
    assert_eq!(callback.listen.current, 0.25);
    assert_eq!(callback.volume.current, 1.0);
    assert_eq!(callback.ramp_frames, 441);
}
