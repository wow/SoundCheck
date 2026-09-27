//! The track held for the grid view: waveform bins equal a brute-force min/max at every level,
//! bins the decoder has not reached are marked, long files fold to mono, float overs are scaled
//! down, and reads for the player copy exactly the decoded samples.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use sc_core::testsig;
use sc_core::{AudioSpec, DbFs};
use sc_engine::track::{CHUNK_FRAMES, NOT_DECODED};
use sc_engine::{Track, TrackProgress};

/// Seeded noise as a 16-bit WAV with `channels`, and its samples.
fn noise_wav(dir: &Path, seconds: f64, channels: u16) -> (PathBuf, Vec<i16>) {
    let spec = AudioSpec {
        sample_rate: 44_100,
        channels,
    };
    let buf = testsig::seeded_noise(spec, 7, 0.8, seconds);
    let path = dir.join(format!("noise-{channels}.wav"));
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    let mut samples = Vec::with_capacity(buf.data.len());
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * 32_767.0).round() as i16;
        samples.push(q);
        w.write_sample(q).unwrap();
    }
    w.finalize().unwrap();
    (path, samples)
}

/// Opens `path` and waits until decoding ends; returns the track and the final report.
fn decoded(path: &Path, peak: DbFs, mono_after_s: f64) -> (Track, TrackProgress) {
    let (tx, rx) = mpsc::channel();
    let track = Track::open_with(path, peak, mono_after_s, move |p| {
        let _ = tx.send(p);
    })
    .unwrap();
    let last = rx
        .iter()
        .find(|p| !matches!(p, TrackProgress::Decoded(_)))
        .unwrap();
    (track, last)
}

/// Reports up to and including the final one.
fn until_end(rx: &mpsc::Receiver<TrackProgress>) -> Vec<TrackProgress> {
    let mut all = Vec::new();
    for p in rx {
        let end = !matches!(p, TrackProgress::Decoded(_));
        all.push(p);
        if end {
            break;
        }
    }
    all
}

/// Brute-force folded min/max per bin over interleaved `samples`.
fn brute(samples: &[i16], channels: usize, spb: usize) -> Vec<i16> {
    samples
        .chunks(spb * channels)
        .flat_map(|bin| [*bin.iter().min().unwrap(), *bin.iter().max().unwrap()])
        .collect()
}

#[test]
fn waveform_bins_equal_a_brute_force_min_max_at_every_level() {
    let dir = tempfile::tempdir().unwrap();
    let (path, samples) = noise_wav(dir.path(), 7.3, 2);
    let (track, last) = decoded(&path, DbFs(-2.0), 1200.0);
    let frames = samples.len() / 2;
    assert!(
        matches!(last, TrackProgress::Done(n) if usize::try_from(n).unwrap() == frames),
        "{last:?}"
    );
    assert_eq!(usize::try_from(track.decoded_frames()).unwrap(), frames);
    let mut spb = 8;
    while spb <= 262_144 {
        let bins = frames.div_ceil(spb) as u64;
        let got = track.peaks(spb as u64, 0, bins.min(16_384)).unwrap();
        let want = brute(&samples, 2, spb);
        assert_eq!(got.len(), want.len().min(2 * 16_384), "{spb}");
        assert_eq!(got, want[..got.len()], "{spb} frames per bin");
        spb *= 2;
    }
    // A window in the middle, and bins past the end.
    let got = track.peaks(64, 1000, 10).unwrap();
    assert_eq!(got, brute(&samples[2 * 64_000..2 * 64_640], 2, 64));
    let past = track.peaks(1024, 10_000, 2).unwrap();
    assert_eq!(past, vec![0, 0, 0, 0]);
    assert!(track.peaks(100, 0, 1).is_err(), "not a power of two");
    assert!(track.peaks(4, 0, 1).is_err(), "finer than the finest level");
    assert!(track.peaks(64, 0, 20_000).is_err(), "too many bins");
}

#[test]
fn reads_copy_the_decoded_samples_across_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let (path, samples) = noise_wav(dir.path(), 4.0, 2);
    let (track, _) = decoded(&path, DbFs(-2.0), 1200.0);
    let from = CHUNK_FRAMES as u64 - 100;
    let mut out = vec![0_i16; 2 * 300];
    assert_eq!(track.read(from, &mut out), 300);
    assert_eq!(
        out,
        samples[2 * (CHUNK_FRAMES - 100)..2 * (CHUNK_FRAMES + 200)]
    );
    let end = (samples.len() / 2) as u64;
    assert_eq!(track.read(end - 10, &mut out), 10, "stops at the end");
}

#[test]
fn a_reader_during_decoding_sees_only_decoded_bins_or_the_mark() {
    let dir = tempfile::tempdir().unwrap();
    let (path, samples) = noise_wav(dir.path(), 20.0, 2);
    let want = brute(&samples, 2, 1024);
    let (tx, rx) = mpsc::channel();
    let track = Arc::new(
        Track::open(&path, DbFs(-2.0), move |p| {
            let _ = tx.send(p);
        })
        .unwrap(),
    );
    let reader = {
        let track = Arc::clone(&track);
        std::thread::spawn(move || {
            let mut partial_views = 0;
            loop {
                let done = track.is_done();
                let got = track.peaks(1024, 0, 862).unwrap();
                let mut marked = false;
                for (i, pair) in got.chunks(2).enumerate() {
                    if pair == [NOT_DECODED, NOT_DECODED] {
                        marked = true;
                    } else {
                        assert!(!marked, "bin {i} decoded after a marked one");
                        assert_eq!(pair, &want[2 * i..2 * i + 2], "bin {i}");
                    }
                }
                partial_views += usize::from(marked);
                if done {
                    assert!(!marked);
                    return partial_views;
                }
            }
        })
    };
    let reports = until_end(&rx);
    assert!(
        matches!(reports.last(), Some(TrackProgress::Done(_))),
        "{reports:?}"
    );
    let decoded_reports = reports
        .iter()
        .filter(|p| matches!(p, TrackProgress::Decoded(_)));
    assert!(
        decoded_reports.count() <= 5,
        "progress at most every 100 ms: {reports:?}"
    );
    reader.join().unwrap();
}

#[test]
fn a_truncated_file_keeps_its_decoded_part_and_marks_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (path, samples) = noise_wav(dir.path(), 6.0, 2);
    // Cut the data to about half: the header still declares six seconds.
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    let (track, last) = decoded(&path, DbFs(-2.0), 1200.0);
    assert!(matches!(last, TrackProgress::Failed(_)), "{last:?}");
    assert!(track.is_done() && track.failed());
    let decoded = usize::try_from(track.decoded_frames()).unwrap();
    assert!(
        decoded > 100_000 && decoded < samples.len() / 2,
        "{decoded}"
    );
    let got = track.peaks(1024, 0, 300).unwrap();
    let whole = decoded / 1024;
    assert_eq!(
        got[..2 * whole],
        brute(&samples[..2 * whole * 1024], 2, 1024)[..]
    );
    assert_eq!(
        &got[2 * (whole + 1)..2 * (whole + 2)],
        &[NOT_DECODED, NOT_DECODED]
    );
}

#[test]
fn dropping_the_track_stops_its_decoding() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = noise_wav(dir.path(), 60.0, 2);
    let (tx, rx) = mpsc::channel::<TrackProgress>();
    let track = Track::open(&path, DbFs(-2.0), move |p| {
        let _ = tx.send(p);
    })
    .unwrap();
    drop(track);
    // The thread ends and drops the callback (and its sender) without reporting a failure.
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(TrackProgress::Decoded(_)) => {}
            Ok(other) => assert!(matches!(other, TrackProgress::Done(_)), "{other:?}"),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("the decoding thread still runs"),
        }
    }
}

#[test]
fn a_24_bit_source_rounds_to_16_bits_and_full_scale_is_never_the_mark() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep.wav");
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    let values: Vec<i32> = (0..44_100)
        .map(|i| match i % 5 {
            0 => -8_388_608,
            1 => 8_388_607,
            2 => 12_345 * 7,
            3 => -383,
            _ => 128,
        })
        .collect();
    for &v in &values {
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
    let (track, _) = decoded(&path, DbFs(0.0), 1200.0);
    let mut out = vec![0_i16; 5];
    assert_eq!(track.read(0, &mut out), 5);
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let want: Vec<i16> = values[..5]
        .iter()
        .map(|&v| (v as f32 / 256.0).round().clamp(-32_767.0, 32_767.0) as i16)
        .collect();
    assert_eq!(out, want);
    assert_eq!(out[0], -32_767, "full scale is stored one step inside");
    assert!(!track.peaks(8, 0, 5000).unwrap().contains(&NOT_DECODED));
}

#[test]
fn a_long_stereo_file_is_held_as_mono() {
    let dir = tempfile::tempdir().unwrap();
    let (path, samples) = noise_wav(dir.path(), 3.0, 2);
    let (track, _) = decoded(&path, DbFs(-2.0), 2.0);
    assert_eq!(track.channels(), 1);
    // Across a chunk boundary, exactly the average of the two channels.
    let mono: Vec<i16> = samples
        .chunks(2)
        .map(|f| {
            let m = f32::midpoint(f32::from(f[0]), f32::from(f[1]));
            #[allow(clippy::cast_possible_truncation)] // the average of two i16 values
            let rounded = m.round() as i16;
            rounded
        })
        .collect();
    let from = CHUNK_FRAMES - 80;
    let got = track.peaks(8, (from / 8) as u64, 20).unwrap();
    assert_eq!(got, brute(&mono[from / 8 * 8..(from / 8 + 20) * 8], 1, 8));
    let mut out = vec![0_i16; 4];
    assert_eq!(track.read(1000, &mut out), 4);
    for (i, &m) in out.iter().enumerate() {
        let l = f32::from(samples[2 * (1000 + i)]);
        let r = f32::from(samples[2 * (1000 + i) + 1]);
        assert!(
            (f32::from(m) - f32::midpoint(l, r)).abs() <= 1.0,
            "{m} vs {l}, {r}"
        );
    }
}

#[test]
fn a_float_file_with_overs_is_scaled_down() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hot.wav");
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..44_100 {
        w.write_sample(if i % 100 == 0 { 2.0_f32 } else { -0.5 })
            .unwrap();
    }
    w.finalize().unwrap();
    // +6.02 dBFS sample peak.
    let (track, _) = decoded(&path, DbFs(6.0206), 1200.0);
    assert!((track.scale() - 0.5).abs() < 1e-4, "{}", track.scale());
    let bins = track.peaks(44_100_u64.next_power_of_two(), 0, 1).unwrap();
    assert_eq!(bins, vec![-8192, 32_767]);
}
