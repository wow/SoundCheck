//! Throughput of the IFF render: a 6-minute 44.1 kHz stereo 24-bit WAV (about 95 MB) on disk,
//! -3.2 dB applied into a new file in the same temporary folder (read, gain, round, write;
//! the page cache is warm after the first iteration).
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use std::io::Write;
use std::path::Path;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_core::RenderRequest;
use sc_io::render::apply_iff;

const RATE_HZ: u32 = 44_100;
const SECONDS: u32 = 360;

/// A plain-PCM 24-bit stereo WAV with deterministic pseudo-random samples (xorshift32) at
/// about -6 dBFS peak.
fn write_wav(path: &Path) {
    let frames = RATE_HZ * SECONDS;
    let data_len = frames * 6;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).expect("temp file"));
    let mut head = Vec::new();
    head.extend_from_slice(b"RIFF");
    head.extend_from_slice(&(36 + data_len).to_le_bytes());
    head.extend_from_slice(b"WAVEfmt ");
    head.extend_from_slice(&16_u32.to_le_bytes());
    head.extend_from_slice(&1_u16.to_le_bytes());
    head.extend_from_slice(&2_u16.to_le_bytes());
    head.extend_from_slice(&RATE_HZ.to_le_bytes());
    head.extend_from_slice(&(RATE_HZ * 6).to_le_bytes());
    head.extend_from_slice(&6_u16.to_le_bytes());
    head.extend_from_slice(&24_u16.to_le_bytes());
    head.extend_from_slice(b"data");
    head.extend_from_slice(&data_len.to_le_bytes());
    out.write_all(&head).expect("write");
    let mut x = 0x9E37_79B9_u32;
    for _ in 0..frames * 2 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        // Halve the 24-bit value: about -6 dBFS peak.
        let s = x.cast_signed() >> 9;
        out.write_all(&s.to_le_bytes()[..3]).expect("write");
    }
    out.flush().expect("flush");
}

fn bench_apply(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("in.wav");
    let output = dir.path().join("out.wav");
    write_wav(&input);
    let bytes = std::fs::metadata(&input).expect("input").len();
    let req = RenderRequest {
        gain_db: -3.2,
        ..RenderRequest::default()
    };
    let mut group = c.benchmark_group("render");
    group.throughput(Throughput::Bytes(bytes));
    group.sample_size(10);
    group.bench_function("apply_iff -3.2 dB 6 min 44.1k stereo 24-bit", |b| {
        b.iter(|| {
            let _ = std::fs::remove_file(&output);
            apply_iff(&input, &output, &req).expect("render")
        });
    });
    group.finish();
}

criterion_group!(benches, bench_apply);
criterion_main!(benches);
