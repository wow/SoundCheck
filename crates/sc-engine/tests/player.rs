//! The click player through a null sink: the test plays the audio thread's part by calling the
//! callback and collecting what it writes. Clicks land on the grid lines (exactly at the track's
//! rate, within 2 samples after resampling), output never exceeds full scale, a seek drops
//! queued audio, pause holds the position, the end stops output, and underruns are counted only
//! when the ring runs dry while playing.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

use sc_core::analysis::{Alternatives, Confidence, Grid, Meter, Verdict};
use sc_core::{Bpm, DbFs, SampleIndex};
use sc_engine::player::click::{Accent, Clicks};
use sc_engine::player::{Callback, Feeder, Renderer, Shared, ring};
use sc_engine::{Track, TrackProgress};

/// A 16-bit stereo WAV at `rate` whose frame `n` is `f(n)` on both channels.
fn wav(path: &Path, rate: u32, frames: u32, f: impl Fn(u32) -> i16) {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..frames {
        w.write_sample(f(n)).unwrap();
        w.write_sample(f(n)).unwrap();
    }
    w.finalize().unwrap();
}

fn track(path: &Path) -> Arc<Track> {
    let (tx, rx) = mpsc::channel();
    let t = Track::open(path, DbFs(-1.0), move |p| {
        if !matches!(p, TrackProgress::Decoded(_)) {
            let _ = tx.send(());
        }
    })
    .unwrap();
    rx.recv().unwrap();
    Arc::new(t)
}

fn grid(bpm: f64, anchor: u64, meter: Meter) -> Arc<Grid> {
    Arc::new(Grid {
        anchor: SampleIndex(anchor),
        bpm: Bpm(bpm),
        meter,
        meter_runner_up: None,
        first_downbeat_index: 0,
        phrase_len_bars: 8,
        segments: Vec::new(),
        residual_p95_ms: 0.0,
        residual_max_ms: 0.0,
        local_bpm_range: 0.0,
        drift_ppm: 0.0,
        verdict: Verdict::Static,
        confidence: Confidence::Green,
        reasons: Vec::new(),
        alternatives: Alternatives::default(),
    })
}

struct Rig {
    feeder: Feeder,
    callback: Callback,
    shared: Arc<Shared>,
}

fn rig(track: Arc<Track>, gain: f64, out_rate: u32) -> Rig {
    let shared = Arc::new(Shared::default());
    let (producer, consumer) = ring(out_rate, 2);
    let renderer = Renderer::new(track, DbFs(gain), out_rate, 2).unwrap();
    Rig {
        feeder: Feeder::new(renderer, producer, Arc::clone(&shared), out_rate, 2),
        callback: Callback::new(consumer, Arc::clone(&shared), 2),
        shared,
    }
}

impl Rig {
    /// Plays `frames` output frames in device buffers of 256 frames; returns the left channel.
    fn play(&mut self, frames: usize) -> Vec<f32> {
        let mut left = Vec::with_capacity(frames);
        let mut buffer = vec![0.0_f32; 512];
        while left.len() < frames {
            self.feeder.pump();
            self.callback.fill(&mut buffer);
            left.extend(buffer.iter().step_by(2).copied());
        }
        left.truncate(frames);
        left
    }
}

/// First sample of each click: the first non-zero sample after at least 100 silent ones, minus
/// one (a click's first sample is exactly zero).
fn onsets(left: &[f32]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut silent = 100;
    for (i, &x) in left.iter().enumerate() {
        if x.abs() > 1e-4 {
            if silent >= 100 {
                found.push(i - 1);
            }
            silent = 0;
        } else {
            silent += 1;
        }
    }
    found
}

#[test]
fn clicks_land_on_the_grid_lines_with_their_accents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("silence.wav");
    wav(&path, 48_000, 4 * 48_000, |_| 0);
    let mut r = rig(track(&path), 0.0, 48_000);
    // 120 BPM, bar 1 at 0.5 s: a pickup line at 0, then every 24,000 samples.
    r.feeder
        .set_grid(Some(grid(120.0, 24_000, Meter::four_four())));
    r.feeder.set_click(true);
    r.feeder.play();
    let left = r.play(4 * 48_000);
    let found = onsets(&left);
    let lines: Vec<usize> = (0..8).map(|i| i * 24_000).collect();
    assert_eq!(found, lines);
    let peak = |at: usize| {
        left[at..at + 240]
            .iter()
            .fold(0.0_f32, |m, x| m.max(x.abs()))
    };
    assert!((peak(24_000) - 0.5).abs() < 0.03, "bar: {}", peak(24_000));
    assert!(
        (peak(48_000) - 0.316).abs() < 0.03,
        "beat 2: {}",
        peak(48_000)
    );
    assert!(
        (peak(0) - 0.316).abs() < 0.03,
        "the pickup is beat 4: {}",
        peak(0)
    );
    assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 0);
}

#[test]
fn resampled_clicks_stay_within_two_samples_of_their_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("silence-44k.wav");
    wav(&path, 44_100, 3 * 44_100, |_| 0);
    let g = grid(128.0, 11_025, Meter::four_four());
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.set_grid(Some(Arc::clone(&g)));
    r.feeder.set_click(true);
    r.feeder.play();
    let left = r.play(3 * 48_000);
    let spb = g.samples_per_beat(44_100);
    // Where a burst first reaches 0.05: after resampling, and at the track's own rate (the
    // click's own shape), mapped to the output rate.
    let crossing = |at: usize| (at..at + 400).find(|&i| left[i].abs() > 0.05).unwrap();
    let clicks = Clicks::new(44_100);
    let rise = |accent: Accent| {
        let k = clicks
            .sound(accent)
            .iter()
            .position(|x| x.abs() > 0.05)
            .unwrap();
        f64::from(u32::try_from(k).unwrap())
    };
    for i in 0..6_u32 {
        let accent = Accent::of(&g.meter, usize::try_from(i % 4).unwrap());
        let line = (11_025.0 + f64::from(i) * spb).round();
        let expected = (line + rise(accent)) * 48_000.0 / 44_100.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = crossing(expected as usize - 50);
        #[allow(clippy::cast_precision_loss)]
        let off = at as f64 - expected;
        assert!(off.abs() <= 2.0, "click {i}: {at} vs {expected:.1}");
    }
}

#[test]
fn output_never_exceeds_full_scale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("square.wav");
    wav(&path, 48_000, 48_000, |n| {
        if (n / 50) % 2 == 0 { 32_767 } else { -32_767 }
    });
    let mut r = rig(track(&path), 6.0, 48_000);
    r.feeder.set_grid(Some(grid(140.0, 0, Meter::four_four())));
    r.feeder.set_click(true);
    r.feeder.play();
    let left = r.play(48_000);
    let peak = left.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
    assert!(peak <= 1.0, "{peak}");
    assert!(peak > 0.9, "the music is there: {peak}");
}

#[test]
fn a_seek_drops_queued_audio_and_pause_holds_the_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ramp.wav");
    #[allow(clippy::cast_possible_truncation)]
    wav(&path, 48_000, 3 * 48_000, |n| (n % 30_000) as i16);
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.play();
    // 19 device buffers of 256 frames.
    let first = r.play(19 * 256);
    assert!((first[100] - 100.0 / 32_768.0).abs() < 1e-6);
    assert_eq!(r.feeder.state().position, SampleIndex(19 * 256));

    r.feeder.seek(96_000);
    let after = r.play(2000);
    // The first device buffer after the seek empties the ring (silence); the new audio follows.
    let start = after.iter().position(|&x| x != 0.0).unwrap();
    let expected = f32::from(i16::try_from(96_000 % 30_000).unwrap()) / 32_768.0;
    assert!(
        (after[start] - expected).abs() < 1e-6,
        "{} vs {expected}",
        after[start]
    );
    assert!(start <= 256, "one device buffer at most: {start}");

    r.feeder.pause();
    let held = r.feeder.state().position;
    let silence = r.play(1000);
    assert!(silence.iter().all(|&x| x == 0.0));
    assert_eq!(r.feeder.state().position, held);
    assert!(!r.feeder.state().playing);
}

#[test]
fn the_end_stops_output_and_only_a_dry_ring_counts_as_an_underrun() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("short.wav");
    wav(&path, 48_000, 12_000, |_| 1000);
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.play();
    let _ = r.play(20_000);
    assert!(!r.feeder.state().playing, "stopped at the end");
    assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 0);

    // Playing with nothing queued and no end in sight: an underrun per device buffer.
    let path = dir.path().join("long.wav");
    wav(&path, 48_000, 48_000, |_| 1000);
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.play();
    let mut buffer = vec![0.0_f32; 512];
    r.callback.fill(&mut buffer);
    assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 1);
    assert!(buffer.iter().all(|&x| x == 0.0));
}

/// The device player's thread starts, takes commands without opening a device until asked to
/// play, and ends when the player is dropped. (No test opens a real audio device.)
#[cfg(feature = "playback")]
#[test]
fn the_device_player_opens_nothing_until_it_plays() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quiet.wav");
    wav(&path, 44_100, 44_100, |_| 0);
    let player = sc_engine::player::Player::new().unwrap();
    player.load(track(&path), DbFs(0.0));
    player.set_click(true);
    player.set_grid(Some(grid(120.0, 0, Meter::four_four())));
    player.seek(SampleIndex(1000));
    std::thread::sleep(std::time::Duration::from_millis(30));
    let status = player.status();
    assert_eq!(status.state, None, "no stream before play");
    assert_eq!(status.error, None);
    drop(player);
}
