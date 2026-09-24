//! Throughput of the sample-peak scan on 60 s of 44.1 kHz stereo.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sc_core::{AudioSpec, testsig};

fn bench_peak(c: &mut Criterion) {
    let buf = testsig::seeded_noise(AudioSpec::CD, 1, 0.5, 60.0);
    c.bench_function("sample_peak 60s stereo", |b| {
        b.iter(|| sc_analysis::sample_peak(black_box(&buf.data)));
    });
}

criterion_group!(benches, bench_peak);
criterion_main!(benches);
