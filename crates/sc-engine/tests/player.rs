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
use sc_engine::player::{Callback, Feeder, Meters, Renderer, Shared, ring};
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
    channels: usize,
}

fn rig(track: Arc<Track>, gain: f64, out_rate: u32) -> Rig {
    rig_with(track, gain, out_rate, 2)
}

fn rig_with(track: Arc<Track>, gain: f64, out_rate: u32, channels: u16) -> Rig {
    let shared = Arc::new(Shared::default());
    let (producer, consumer) = ring(out_rate, channels);
    let renderer = Renderer::new(track, out_rate, channels).unwrap();
    let meters = Arc::new(Meters::new());
    let mut feeder = Feeder::new(
        renderer,
        producer,
        Arc::clone(&shared),
        meters,
        out_rate,
        channels,
    )
    .unwrap();
    feeder.set_planned_gain(DbFs(gain));
    Rig {
        feeder,
        callback: Callback::new(consumer, Arc::clone(&shared), channels, out_rate),
        shared,
        channels: usize::from(channels),
    }
}

impl Rig {
    /// Plays `frames` output frames in device buffers of 256 frames; returns every channel,
    /// interleaved.
    fn play_all(&mut self, frames: usize) -> Vec<f32> {
        let ch = self.channels;
        let mut all = Vec::with_capacity(frames * ch);
        let mut buffer = vec![0.0_f32; 256 * ch];
        while all.len() < frames * ch {
            self.feeder.pump();
            self.callback.fill(&mut buffer);
            all.extend_from_slice(&buffer);
        }
        all.truncate(frames * ch);
        all
    }

    /// As [`Rig::play_all`], the first channel only.
    fn play(&mut self, frames: usize) -> Vec<f32> {
        let ch = self.channels;
        self.play_all(frames).iter().step_by(ch).copied().collect()
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

    // Waiting for the first audio is not an underrun; a ring that runs dry once audio flows,
    // with no end in sight, is one per device buffer.
    let path = dir.path().join("long.wav");
    wav(&path, 48_000, 48_000, |_| 1000);
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.play();
    let mut buffer = vec![0.0_f32; 512];
    r.callback.fill(&mut buffer);
    assert_eq!(
        r.shared.underruns.load(Ordering::Relaxed),
        0,
        "not primed yet"
    );
    r.feeder.pump();
    for _ in 0..40 {
        r.callback.fill(&mut buffer);
    }
    assert!(r.shared.underruns.load(Ordering::Relaxed) > 0);
    assert!(buffer.iter().all(|&x| x == 0.0));
}

#[test]
fn a_line_just_before_a_block_boundary_still_clicks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("silence.wav");
    wav(&path, 48_000, 3 * 48_000, |_| 0);
    let mut r = rig(track(&path), 0.0, 48_000);
    // One line every 20,479.7 samples: the second rounds onto 20,480, the first sample of
    // block 20 (blocks of 1,024).
    let bpm = 60.0 * 48_000.0 / 20_479.7;
    r.feeder.set_grid(Some(grid(bpm, 0, Meter::four_four())));
    r.feeder.set_click(true);
    r.feeder.play();
    let left = r.play(3 * 48_000);
    assert_eq!(
        onsets(&left),
        vec![0, 20_480, 40_959, 61_439, 81_919, 102_399, 122_878, 143_358]
    );
}

#[test]
fn right_after_a_seek_the_position_is_the_seek_point_and_no_underrun_is_counted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");
    wav(&path, 48_000, 6 * 48_000, |n| {
        if n % 2 == 0 { 3000 } else { -3000 }
    });
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.play();
    let _ = r.play(5 * 48_000);
    r.feeder.seek(1000);
    assert_eq!(
        r.feeder.state().position,
        SampleIndex(1000),
        "before the callback acknowledges"
    );
    let _ = r.play(4800);
    assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 0);
}

#[test]
fn resampled_output_stays_within_full_scale_and_the_end_is_not_cut() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("square-44k.wav");
    wav(&path, 44_100, 22_050, |n| {
        if (n / 40) % 2 == 0 { 32_767 } else { -32_767 }
    });
    let mut r = rig(track(&path), 6.0, 48_000);
    r.feeder.play();
    let left = r.play(48_000);
    let peak = left.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
    assert!(peak <= 1.0, "{peak}");
    // Half a second at 44.1 kHz is 24,000 frames at 48 kHz, all of them heard.
    let last = left.iter().rposition(|&x| x.abs() > 0.5).unwrap();
    assert!(last >= 23_990, "the tail stops at {last}");
}

#[test]
fn music_and_clicks_use_the_first_two_channels_and_mono_hears_both() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("level.wav");
    wav(&path, 48_000, 48_000, |_| 8192);
    let mut r = rig_with(track(&path), 0.0, 48_000, 4);
    r.feeder.set_grid(Some(grid(120.0, 0, Meter::four_four())));
    r.feeder.set_click(true);
    r.feeder.play();
    let all = r.play_all(24_000);
    assert!(
        all.chunks(4).all(|f| f[2..].iter().all(|&x| x == 0.0)),
        "the third and fourth channels stay silent"
    );
    assert!(
        all.chunks(4).any(|f| (f[0] - 0.125).abs() > 0.01),
        "clicks on the first channel"
    );
    let mut mono = rig_with(track(&path), 0.0, 48_000, 1);
    mono.feeder.play();
    let heard = mono.play(4800);
    assert!((heard[1000] - 0.25).abs() < 1e-6, "{}", heard[1000]);
}

#[test]
fn an_edit_while_paused_is_heard_from_the_first_frame_played() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("silence.wav");
    wav(&path, 48_000, 4 * 48_000, |_| 0);
    let mut r = rig(track(&path), 0.0, 48_000);
    r.feeder.set_click(true);
    r.feeder.play();
    let _ = r.play(256 * 40);
    r.feeder.pause();
    let _ = r.play(512);
    let at = r.feeder.state().position.0;
    // A new grid with a line 100 samples after the paused position.
    r.feeder
        .set_grid(Some(grid(120.0, at + 100, Meter::four_four())));
    r.feeder.play();
    let left = r.play(2000);
    let first = onsets(&left)[0];
    assert!(
        first.abs_diff(100) <= 256,
        "the new line is heard at {first}"
    );
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
    let paused_at_the_seek = sc_engine::player::PlayerState {
        playing: false,
        position: SampleIndex(1000),
        underruns: 0,
    };
    assert_eq!(
        status.state,
        Some(paused_at_the_seek),
        "no stream before play"
    );
    assert_eq!(status.error, None);
    drop(player);
}
