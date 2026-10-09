//! What the user hears and what the meters show, through a null sink (the test plays the audio
//! thread's part): switching versions is heard within one device buffer along a 10 ms ramp, level
//! matching plays the processed version at the original level, the monitor volume scales what is
//! heard by exactly its dB and changes no meter, OUT reads the planned gain above IN, and the meter
//! frame for the heard position lies within one block of it, also after a seek.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

use sc_core::ipc::{Listen, MeterFrame, Version};
use sc_core::{DbFs, SampleIndex};
use sc_engine::player::{BLOCK_FRAMES, Callback, Feeder, Meters, Renderer, Shared, ring};
use sc_engine::{Track, TrackProgress};

const RATE: u32 = 48_000;
/// Device buffer, frames.
const BUFFER: usize = 256;

/// A 16-bit stereo WAV at 48 kHz whose frame `n` is `f(n)` on both channels.
fn wav(path: &Path, frames: u32, f: impl Fn(u32) -> i16) {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
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

/// A 1 kHz sine of amplitude `amp` (of full scale).
#[allow(clippy::cast_possible_truncation)]
fn sine(amp: f64) -> impl Fn(u32) -> i16 {
    move |n| {
        let phase = 2.0 * std::f64::consts::PI * 1_000.0 * f64::from(n) / f64::from(RATE);
        (amp * phase.sin() * 32_767.0).round() as i16
    }
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

struct Rig {
    feeder: Feeder,
    callback: Callback,
    shared: Arc<Shared>,
}

fn rig(track: Arc<Track>, planned: f64) -> Rig {
    let shared = Arc::new(Shared::default());
    let (producer, consumer) = ring(RATE, 2);
    let renderer = Renderer::new(track, RATE, 2).unwrap();
    let meters = Arc::new(Meters::new());
    let mut feeder = Feeder::new(renderer, producer, Arc::clone(&shared), meters, RATE, 2).unwrap();
    feeder.set_planned_gain(DbFs(planned));
    Rig {
        feeder,
        callback: Callback::new(consumer, Arc::clone(&shared), 2, RATE),
        shared,
    }
}

impl Rig {
    /// Plays `buffers` device buffers; returns the left channel.
    fn play(&mut self, buffers: usize) -> Vec<f32> {
        let mut left = Vec::with_capacity(buffers * BUFFER);
        let mut buffer = [0.0_f32; 2 * BUFFER];
        for _ in 0..buffers {
            self.feeder.pump();
            self.callback.fill(&mut buffer);
            left.extend(buffer.iter().step_by(2));
        }
        left
    }

    /// The meter frame for the position heard now.
    fn heard_meter(&self) -> (SampleIndex, Option<MeterFrame>) {
        let heard = self.feeder.state().position;
        (heard, self.feeder.meter_at(heard))
    }
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

#[test]
fn a_version_switch_is_heard_in_the_next_buffer_and_settles_in_ten_milliseconds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dc.wav");
    wav(&path, 2 * RATE, |_| 8_192);
    // -6.02 dB: the processed version plays at half the original's level.
    let mut r = rig(track(&path), db(0.5));
    r.feeder.play();
    let before = r.play(20);
    assert!((before[before.len() - 1] - 0.125).abs() < 1e-6);

    r.feeder.set_listen(Listen {
        version: Version::Original,
        matched: false,
    });
    let after = r.play(4);
    let step = (0.25 - 0.125) / 480.0;
    assert!(
        (after[0] - (0.125 + step)).abs() < 1e-6,
        "the first frame of the next buffer moves: {}",
        after[0]
    );
    assert!(
        after[..480].windows(2).all(|w| w[1] > w[0]),
        "a rising line"
    );
    assert!(
        (after[478] - 0.25).abs() > step / 2.0,
        "not there a frame early"
    );
    assert!(
        (after[479] - 0.25).abs() < 1e-6,
        "there after 480 frames (10 ms)"
    );
    assert!(after[479..].iter().all(|&x| (x - 0.25).abs() < 1e-6));
}

#[test]
fn level_matching_plays_the_processed_version_at_the_original_level() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dc.wav");
    wav(&path, 2 * RATE, |_| 8_192);
    let mut r = rig(track(&path), -3.7);
    r.feeder.play();
    let mut level = |listen: Listen| {
        r.feeder.set_listen(listen);
        let heard = r.play(10);
        f64::from(heard[heard.len() - 1])
    };
    let original = level(Listen {
        version: Version::Original,
        matched: false,
    });
    let processed = level(Listen::default());
    let matched = level(Listen {
        version: Version::Processed,
        matched: true,
    });
    let original_matched = level(Listen {
        version: Version::Original,
        matched: true,
    });
    assert!((db(processed / original) + 3.7).abs() < 0.01);
    assert!(db(matched / original).abs() < 0.01, "matched = original");
    assert!(db(original_matched / original).abs() < 0.01);
}

#[test]
fn the_volume_scales_what_is_heard_by_its_db_and_no_meter_moves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sine.wav");
    wav(&path, 4 * RATE, sine(0.5));
    let mut r = rig(track(&path), -2.0);
    r.feeder.play();
    let peak = |x: &[f32]| f64::from(x.iter().fold(0.0_f32, |m, v| m.max(v.abs())));
    let full = r.play(100);
    let (_, unmuted) = r.heard_meter();
    let unmuted = unmuted.expect("a reading after 0.5 s");
    for volume in [-12.0, -0.5, -60.0] {
        r.feeder.set_volume(Some(DbFs(volume)));
        let heard = r.play(20);
        let change = db(peak(&heard[BUFFER * 4..]) / peak(&full[full.len() - 4_800..]));
        assert!(
            (change - volume).abs() < 0.01,
            "{volume} dB heard as {change}"
        );
        let (_, frame) = r.heard_meter();
        let frame = frame.expect("a reading");
        assert_eq!(
            (frame.in_peak, frame.out_peak),
            (unmuted.in_peak, unmuted.out_peak),
            "the meters show the signal, not the speaker"
        );
        let drift = frame.in_momentary.unwrap().0 - unmuted.in_momentary.unwrap().0;
        assert!(drift.abs() < 0.01, "momentary moved by {drift}");
    }
    r.feeder.set_volume(None);
    let muted = r.play(10);
    assert!(muted[BUFFER * 2..].iter().all(|&x| x == 0.0), "muted");
    assert!(
        r.heard_meter().1.unwrap().in_peak.is_some(),
        "still metered"
    );
}

#[test]
fn out_reads_the_planned_gain_above_in_whatever_is_heard() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sine.wav");
    wav(&path, 4 * RATE, sine(0.25));
    let mut r = rig(track(&path), -4.25);
    r.feeder.play();
    let _ = r.play(100);
    let check = |frame: MeterFrame, gain: f64| {
        let peak = frame.out_peak.unwrap().0 - frame.in_peak.unwrap().0;
        let loud = frame.out_momentary.unwrap().0 - frame.in_momentary.unwrap().0;
        assert!((peak - gain).abs() <= 0.01, "peak {peak} vs {gain}");
        assert!((loud - gain).abs() <= 0.01, "momentary {loud} vs {gain}");
    };
    check(r.heard_meter().1.unwrap(), -4.25);
    r.feeder.set_listen(Listen {
        version: Version::Original,
        matched: false,
    });
    let _ = r.play(5);
    check(r.heard_meter().1.unwrap(), -4.25);
    r.feeder.set_planned_gain(DbFs(1.5));
    check(r.heard_meter().1.unwrap(), 1.5);
    // IN is the original: a 0.25 sine reads -12.04 dBTP.
    let in_peak = r.heard_meter().1.unwrap().in_peak.unwrap().0;
    assert!((in_peak - db(0.25)).abs() < 0.05, "{in_peak}");
}

#[test]
fn the_frame_shown_is_within_one_block_of_the_heard_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sine.wav");
    wav(&path, 6 * RATE, sine(0.25));
    let mut r = rig(track(&path), 0.0);
    // A device that plays each buffer 2,000 frames (42 ms) after it is filled.
    r.shared.latency_frames.store(2_000, Ordering::Release);
    r.feeder.play();
    let mut checked = 0;
    for _ in 0..600 {
        let _ = r.play(1);
        let (heard, frame) = r.heard_meter();
        if r.shared.played.load(Ordering::Acquire) <= 2_000 {
            continue;
        }
        let frame = frame.expect("every heard position has a reading");
        let distance = frame.position.0.abs_diff(heard.0);
        assert!(distance <= BLOCK_FRAMES as u64, "{distance} frames apart");
        checked += 1;
    }
    assert!(checked > 500);
}

#[test]
fn a_seek_restarts_the_meters_at_the_new_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sine.wav");
    wav(&path, 6 * RATE, sine(0.25));
    let mut r = rig(track(&path), 0.0);
    r.feeder.play();
    let _ = r.play(150);
    assert!(r.heard_meter().1.unwrap().in_momentary.is_some());
    r.feeder.seek(4 * u64::from(RATE));
    assert_eq!(
        r.heard_meter().1,
        None,
        "nothing measured at the new position yet"
    );
    let _ = r.play(2);
    let (heard, frame) = r.heard_meter();
    let frame = frame.expect("measured once the ring refills");
    assert!(heard.0 >= 4 * u64::from(RATE));
    assert!(frame.position.0.abs_diff(heard.0) <= BLOCK_FRAMES as u64);
    assert_eq!(frame.in_momentary, None, "a fresh 400 ms window");
    assert!(frame.in_peak.is_some());
    // 400 ms later the window is full again.
    let _ = r.play(80);
    assert!(r.heard_meter().1.unwrap().in_momentary.is_some());
}
