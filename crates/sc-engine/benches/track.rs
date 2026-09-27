#![allow(missing_docs)] // criterion_group! generates undocumented items
//! The grid view's track: decoding a six-minute stereo file into memory (budget < 3 s) and one
//! waveform request of 3,000 bins at the beat-zoom and overview levels (budget < 2 ms). A real
//! file can be timed instead of the synthetic WAV with `SC_BENCH_TRACK=<path>`.

use std::path::PathBuf;
use std::sync::mpsc;

use criterion::{Criterion, criterion_group, criterion_main};
use sc_core::{AudioSpec, DbFs, testsig};
use sc_engine::{Track, TrackProgress};

/// Six minutes of stereo noise as a 16-bit WAV in `dir`.
fn six_minutes(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("six-minutes.wav");
    let buf = testsig::seeded_noise(AudioSpec::CD, 3, 0.5, 360.0);
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .expect("create wav");
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * 32_767.0) as i16)
            .expect("write");
    }
    w.finalize().expect("finalize");
    path
}

fn open_decoded(path: &std::path::Path) -> Track {
    let (tx, rx) = mpsc::channel();
    let track = Track::open(path, DbFs(-1.0), move |p| {
        if !matches!(p, TrackProgress::Decoded(_)) {
            let _ = tx.send(());
        }
    })
    .expect("open");
    rx.recv().expect("decoded");
    track
}

fn bench_track(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path =
        std::env::var_os("SC_BENCH_TRACK").map_or_else(|| six_minutes(dir.path()), PathBuf::from);
    let mut group = c.benchmark_group("track");
    group.sample_size(10);
    group.bench_function("decode into memory", |b| {
        b.iter(|| open_decoded(std::hint::black_box(&path)));
    });
    let track = open_decoded(&path);
    group.sample_size(100);
    group.bench_function("peaks 3000 bins at 256 frames", |b| {
        b.iter(|| {
            track
                .peaks(256, std::hint::black_box(1000), 3000)
                .expect("peaks")
        });
    });
    group.bench_function("peaks 3000 bins at 4096 frames", |b| {
        b.iter(|| {
            track
                .peaks(4096, std::hint::black_box(0), 3000)
                .expect("peaks")
        });
    });
    group.finish();
}

criterion_group!(benches, bench_track);
criterion_main!(benches);
