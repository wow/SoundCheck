#![allow(missing_docs)]
//! Loudness throughput on 60 s of stereo noise; the budget is >= 100x real time per core.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_analysis::loudness::measure;
use sc_core::{AudioSpec, testsig};

fn bench_loudness(c: &mut Criterion) {
    let buf = testsig::seeded_noise(AudioSpec::CD, 1, 0.3, 60.0);
    let mut group = c.benchmark_group("loudness");
    group.throughput(Throughput::Elements(buf.frames() as u64));
    group.sample_size(10);
    group.bench_function("measure 60 s stereo", |b| {
        b.iter(|| measure(std::hint::black_box(&buf)).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_loudness);
criterion_main!(benches);
