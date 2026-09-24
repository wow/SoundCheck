//! Throughput of the gain stage on 60 s of 44.1 kHz stereo.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sc_core::{AudioSpec, DbFs, testsig};
use sc_dsp::Gain;

fn bench_gain(c: &mut Criterion) {
    let buf = testsig::seeded_noise(AudioSpec::CD, 1, 0.5, 60.0);
    c.bench_function("gain 60s stereo", |b| {
        b.iter_batched(
            || buf.data.clone(),
            |mut data| {
                Gain::new(DbFs(-3.0)).push(black_box(&mut data));
                data
            },
            criterion::BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_gain);
criterion_main!(benches);
