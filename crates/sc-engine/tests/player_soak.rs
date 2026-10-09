//! A soak of the player on a null sink in real time: a feeding thread pumps the ring every 5 ms
//! and reads the meters at 30 Hz, as the device player's threads do, while this thread plays the
//! device, asking for a 512-frame buffer at 48 kHz on the clock, and switches versions 50 times
//! (with volume moves) spread over the run. No buffer may find the ring short.
//!
//! It runs for 3 s by default; a longer soak (10 minutes):
//!
//! ```text
//! SC_SOAK_S=600 cargo test -p sc-engine --release --test player_soak -- --nocapture
//! ```

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use sc_core::ipc::{Listen, Version};
use sc_core::{DbFs, testsig};
use sc_engine::player::{Callback, Feeder, Meters, Renderer, Shared, ring};
use sc_engine::{Track, TrackProgress};

const OUT_RATE: u32 = 48_000;
const BUFFER_FRAMES: u32 = 512;
const SWITCHES: u32 = 50;

enum Change {
    Listen(Listen),
    Volume(Option<DbFs>),
}

/// Twenty seconds of seeded noise at 44.1 kHz (resampled to the device's 48 kHz as it plays).
fn noise_wav(path: &Path) {
    let audio = testsig::seeded_noise(sc_core::AudioSpec::CD, 11, 0.4, 20.0);
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for s in &audio.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * 32_767.0).round() as i16)
            .unwrap();
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

/// The feeding thread: changes, pumping every 5 ms, a meter reading every 33 ms, the track
/// played again from the start when it ends. Returns how many readings it found.
fn feed(mut feeder: Feeder, changes: &mpsc::Receiver<Change>, stop: &AtomicBool) -> u32 {
    let mut readings = 0;
    let mut next_reading = Instant::now();
    feeder.play();
    while !stop.load(Ordering::Acquire) {
        while let Ok(change) = changes.try_recv() {
            match change {
                Change::Listen(listen) => feeder.set_listen(listen),
                Change::Volume(volume) => feeder.set_volume(volume),
            }
        }
        feeder.pump();
        if feeder.finished() {
            feeder.seek(0);
            feeder.play();
        }
        if Instant::now() >= next_reading {
            next_reading += Duration::from_millis(33);
            let state = feeder.state();
            if state.playing && feeder.meter_at(state.position).is_some() {
                readings += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    readings
}

#[test]
fn fifty_switches_on_a_real_time_null_sink_never_run_the_ring_dry() {
    let seconds: u64 = std::env::var("SC_SOAK_S")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("noise.wav");
    noise_wav(&path);

    let shared = Arc::new(Shared::default());
    let (producer, consumer) = ring(OUT_RATE, 2);
    let renderer = Renderer::new(track(&path), OUT_RATE, 2).unwrap();
    let meters = Arc::new(Meters::new());
    let mut feeder =
        Feeder::new(renderer, producer, Arc::clone(&shared), meters, OUT_RATE, 2).unwrap();
    feeder.set_planned_gain(DbFs(-3.0));
    let mut callback = Callback::new(consumer, Arc::clone(&shared), 2, OUT_RATE);
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let feeding = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || feed(feeder, &rx, &stop))
    };

    let run = Duration::from_secs(seconds);
    let period = Duration::from_secs(1) * BUFFER_FRAMES / OUT_RATE;
    let mut buffer = vec![0.0_f32; BUFFER_FRAMES as usize * 2];
    let start = Instant::now();
    let mut next = start;
    let mut switches = 0;
    while start.elapsed() < run {
        callback.fill(&mut buffer);
        let due = run * (switches + 1) / (SWITCHES + 1);
        if switches < SWITCHES && start.elapsed() >= due {
            switches += 1;
            let version = if switches % 2 == 1 {
                Version::Original
            } else {
                Version::Processed
            };
            tx.send(Change::Listen(Listen {
                version,
                matched: switches % 5 == 0,
            }))
            .unwrap();
            let volume = (switches % 7 != 0).then(|| DbFs(-f64::from(switches % 13)));
            tx.send(Change::Volume(volume)).unwrap();
        }
        next += period;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    stop.store(true, Ordering::Release);
    let readings = feeding.join().unwrap();
    let underruns = shared.underruns.load(Ordering::Relaxed);
    eprintln!("{seconds} s: {switches} switches, {readings} meter readings, {underruns} underruns");
    assert_eq!(switches, SWITCHES);
    assert_eq!(underruns, 0);
    assert!(u64::from(readings) >= seconds * 20, "{readings} readings");
}
