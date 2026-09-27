//! Unit tests of the private parts of `crates/sc-engine/src/player/device.rs`.
use super::*;

#[test]
fn only_a_lost_or_rerouted_device_stops_playback() {
    use cpal::ErrorKind::{
        BackendError, DeviceBusy, DeviceChanged, DeviceNotAvailable, RealtimeDenied,
        StreamInvalidated, Xrun,
    };
    for kind in [
        DeviceNotAvailable,
        DeviceChanged,
        StreamInvalidated,
        BackendError,
    ] {
        assert!(stops_playback(kind), "{kind:?}");
    }
    for kind in [Xrun, DeviceBusy, RealtimeDenied] {
        assert!(!stops_playback(kind), "{kind:?}");
    }
}

/// A short 16-bit stereo WAV of a quiet ramp, decoded to the end.
fn track(dir: &std::path::Path, name: &str) -> Arc<Track> {
    let path = dir.join(name);
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .expect("the temp dir is writable");
    for n in 0..4_410_i16 {
        w.write_sample(n).expect("writes");
        w.write_sample(n).expect("writes");
    }
    w.finalize().expect("writes");
    let (tx, rx) = mpsc::channel();
    let t = Track::open(&path, DbFs(-1.0), move |p| {
        if !matches!(p, crate::track::TrackProgress::Decoded(_)) {
            let _ = tx.send(());
        }
    })
    .expect("the WAV opens");
    rx.recv().expect("decoding ends");
    Arc::new(t)
}

/// Waits (up to 2 s) for the player's thread to publish the status of load `load`.
fn status_of(player: &Player, load: u64) -> Status {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let status = player.status();
        if status.load == load || std::time::Instant::now() > deadline {
            return status;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_status_names_the_load_it_describes() {
    // A new track's position reporter must not take the track before's state for its own:
    // right after `load`, the published status can still describe the track before.
    let dir = tempfile::tempdir().expect("a temp dir");
    let player = Player::new().expect("the player's thread starts");
    assert_eq!(player.status().load, 0);
    let first = player.load(track(dir.path(), "a.wav"), DbFs(0.0));
    player.seek(SampleIndex(2_000));
    let second = player.load(track(dir.path(), "b.wav"), DbFs(0.0));
    assert_eq!((first, second), (1, 2));
    let status = status_of(&player, second);
    assert_eq!(status.load, second);
    // The second track starts paused at its start, whatever the first was doing.
    let state = status.state.expect("a track is loaded");
    assert!(!state.playing);
    assert_eq!(state.position, SampleIndex(0));
}
