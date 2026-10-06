//! Throughput of the FLAC render: a 6-minute 44.1 kHz stereo 24-bit FLAC (a music-like
//! signal: three detuned tones under a slow envelope plus noise 40 dB down, peaking near
//! -6 dBFS) on disk, -3.2 dB applied into a new file in the same temporary folder: decode with
//! CRC and MD5 checks, gain, round, encode, write, then the verifying decode (the page cache is
//! warm after the first iteration).
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_core::RenderRequest;
use sc_io::flac::{FrameEncoder, MARKER, OUTPUT_BLOCK_FRAMES, StreamInfo, block_header};
use sc_io::render::apply_flac;

const RATE_HZ: u32 = 44_100;
const SECONDS: u32 = 360;

/// One interleaved block of the test signal starting at frame `first`.
fn signal(first: u64, frames: usize, noise: &mut u32, out: &mut Vec<i32>) {
    out.clear();
    for n in 0..frames as u64 {
        let t = (first + n) as f64 / f64::from(RATE_HZ);
        let env = 0.6 + 0.4 * (std::f64::consts::TAU * 0.25 * t).sin();
        for (ch, detune) in [(0, 1.0), (1, 1.003)] {
            let tone = [55.0, 220.0, 1760.0]
                .iter()
                .map(|f| (std::f64::consts::TAU * f * detune * t + f64::from(ch)).sin())
                .sum::<f64>()
                / 3.0;
            *noise ^= *noise << 13;
            *noise ^= *noise >> 17;
            *noise ^= *noise << 5;
            let white = f64::from(noise.cast_signed()) / f64::from(i32::MAX);
            let x = 0.5 * env * tone + 0.005 * white;
            // |x| < 0.51, so the 24-bit value fits.
            #[allow(clippy::cast_possible_truncation)]
            out.push((x * f64::from(1 << 23)).round() as i32);
        }
    }
}

fn write_flac(path: &Path) {
    let frames = u64::from(RATE_HZ * SECONDS);
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).expect("temp file"));
    out.write_all(&MARKER).expect("write");
    out.write_all(&block_header(true, 0, 34)).expect("write");
    out.write_all(&[0; 34]).expect("write");
    let mut enc = FrameEncoder::new(RATE_HZ, 2, 24, path).expect("encoder");
    let (mut noise, mut block) = (0x9E37_79B9_u32, Vec::new());
    let mut first = 0;
    while first < frames {
        let n = OUTPUT_BLOCK_FRAMES.min(usize::try_from(frames - first).expect("small"));
        signal(first, n, &mut noise, &mut block);
        enc.push(&block, &mut out).expect("encode");
        first += n as u64;
    }
    let done = enc.finish(&mut out).expect("encode");
    let mut file = out.into_inner().expect("flush");
    let info = StreamInfo {
        min_block: 4096,
        max_block: 4096,
        min_frame: done.min_frame,
        max_frame: done.max_frame,
        sample_rate_hz: RATE_HZ,
        channels: 2,
        bits: 24,
        total_samples: done.total_samples,
        md5: done.md5,
    };
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(8)).expect("seek");
    file.write_all(&info.to_bytes()).expect("write");
}

fn bench_apply(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("in.flac");
    let output = dir.path().join("out.flac");
    write_flac(&input);
    let bytes = u64::from(RATE_HZ * SECONDS) * 2 * 3;
    let req = RenderRequest {
        gain_db: -3.2,
        ..RenderRequest::default()
    };
    let mut group = c.benchmark_group("render");
    group.throughput(Throughput::Bytes(bytes));
    group.sample_size(10);
    group.bench_function("apply_flac -3.2 dB 6 min 44.1k stereo 24-bit", |b| {
        b.iter(|| {
            let _ = std::fs::remove_file(&output);
            apply_flac(&input, &output, &req, &AtomicBool::new(false)).expect("render")
        });
    });
    group.finish();
}

criterion_group!(benches, bench_apply);
criterion_main!(benches);
