//! The whole in-place write transaction on a 6-minute 44.1 kHz stereo 24-bit WAV and FLAC
//! (the music-like signal of `benches/flac.rs`): checks, render into a temp file, sync,
//! verify, backup (an APFS clone on the startup disk), rename, metadata, sidecar, every step
//! journaled and synced. Each iteration restores the source by copy and removes the backup
//! root outside the timed part; the page cache is warm after the first iteration.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_core::RenderRequest;
use sc_io::flac::{FrameEncoder, MARKER, OUTPUT_BLOCK_FRAMES, StreamInfo, block_header};
use sc_io::txn::{TxnOptions, apply_in_place};

const RATE_HZ: u32 = 44_100;
const SECONDS: u32 = 360;

/// One interleaved block of the test signal starting at frame `first`.
fn signal(first: u64, frames: usize, noise: &mut u32, out: &mut Vec<i32>) {
    out.clear();
    for n in 0..frames as u64 {
        // Frame numbers stay below 2^25, exact in f64.
        #[allow(clippy::cast_precision_loss)]
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
    file.seek(SeekFrom::Start(8)).expect("seek");
    file.write_all(&info.to_bytes()).expect("write");
}

fn write_wav(path: &Path) {
    let frames = u64::from(RATE_HZ * SECONDS);
    let data_len = u32::try_from(frames * 6).expect("under 4 GiB");
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).expect("temp file"));
    out.write_all(b"RIFF").expect("write");
    out.write_all(&(36 + data_len).to_le_bytes())
        .expect("write");
    out.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x02\x00")
        .expect("write");
    out.write_all(&RATE_HZ.to_le_bytes()).expect("write");
    out.write_all(&(RATE_HZ * 6).to_le_bytes()).expect("write");
    out.write_all(b"\x06\x00\x18\x00data").expect("write");
    out.write_all(&data_len.to_le_bytes()).expect("write");
    let (mut noise, mut block) = (0x9E37_79B9_u32, Vec::new());
    let mut first = 0;
    while first < frames {
        let n = OUTPUT_BLOCK_FRAMES.min(usize::try_from(frames - first).expect("small"));
        signal(first, n, &mut noise, &mut block);
        for s in &block {
            out.write_all(&s.to_le_bytes()[..3]).expect("write");
        }
        first += n as u64;
    }
    out.flush().expect("flush");
}

/// Times `apply_in_place` on a fresh copy of `source` per iteration.
fn time_txn(source: &Path, dir: &Path, ext: &str, iters: u64) -> Duration {
    let path = dir.join(format!("track.{ext}"));
    let backups = dir.join("backups");
    let req = RenderRequest {
        gain_db: -3.2,
        ..RenderRequest::default()
    };
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let _ = std::fs::remove_dir_all(&backups);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(dir.join(format!("track.{ext}.soundcheck.json")));
        std::fs::copy(source, &path).expect("fresh copy");
        let t = Instant::now();
        apply_in_place(
            &path,
            &req,
            &TxnOptions::new(&backups),
            &AtomicBool::new(false),
        )
        .expect("transaction");
        total += t.elapsed();
    }
    total
}

fn bench_txn(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let (wav, flac) = (base.join("src.wav"), base.join("src.flac"));
    write_wav(&wav);
    write_flac(&flac);
    let work = base.join("work");
    std::fs::create_dir(&work).expect("work folder");
    let mut group = c.benchmark_group("txn");
    group.throughput(Throughput::Bytes(u64::from(RATE_HZ * SECONDS) * 2 * 3));
    group.sample_size(10);
    group.bench_function(
        "apply_in_place WAV -3.2 dB 6 min 44.1k stereo 24-bit",
        |b| {
            b.iter_custom(|iters| time_txn(&wav, &work, "wav", iters));
        },
    );
    group.bench_function(
        "apply_in_place FLAC -3.2 dB 6 min 44.1k stereo 24-bit",
        |b| {
            b.iter_custom(|iters| time_txn(&flac, &work, "flac", iters));
        },
    );
    group.finish();
}

criterion_group!(benches, bench_txn);
criterion_main!(benches);
