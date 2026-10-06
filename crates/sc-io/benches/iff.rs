//! Throughput of the IFF reader: walk + format + streaming PCM read of a 6-minute 44.1 kHz
//! stereo 24-bit WAV (about 95 MB) held in memory.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use std::io::Cursor;
use std::path::Path;

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_io::iff::{self, PcmReader};

const RATE_HZ: u32 = 44_100;
const SECONDS: u32 = 360;

/// A plain-PCM 24-bit stereo WAV with deterministic pseudo-random samples (xorshift32).
fn wav_24bit_stereo() -> Vec<u8> {
    let frames = RATE_HZ * SECONDS;
    let data_len = frames * 6;
    let mut out = Vec::with_capacity(data_len as usize + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&RATE_HZ.to_le_bytes());
    out.extend_from_slice(&(RATE_HZ * 6).to_le_bytes());
    out.extend_from_slice(&6_u16.to_le_bytes());
    out.extend_from_slice(&24_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let mut x = 0x9E37_79B9_u32;
    for _ in 0..frames * 2 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out.extend_from_slice(&x.to_le_bytes()[..3]);
    }
    out
}

fn bench_read(c: &mut Criterion) {
    let bytes = wav_24bit_stereo();
    let path = Path::new("bench.wav");
    let mut group = c.benchmark_group("iff");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.sample_size(10);
    group.bench_function("walk+pcm 6 min 44.1k stereo 24-bit", |b| {
        let mut block = Vec::new();
        b.iter(|| {
            let mut cursor = Cursor::new(black_box(bytes.as_slice()));
            let header = iff::read_header(&mut cursor, path).expect("valid bench file");
            let mut pcm = PcmReader::new(cursor, &header.format, path).expect("consistent format");
            let mut sum = 0_i64;
            while pcm.next_block_int(&mut block).expect("in-memory read") > 0 {
                sum += i64::from(block[0]);
            }
            sum
        });
    });
    group.finish();
}

criterion_group!(benches, bench_read);
criterion_main!(benches);
