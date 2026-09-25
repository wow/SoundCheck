#![allow(missing_docs)]
//! Kick-band filtering, onset detection and resampling on 60 s of mono material.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_core::{AudioSpec, testsig};
use sc_dsp::{KickBand, OnsetDetector, resample::resample_all};

fn bench_primitives(c: &mut Criterion) {
    let mono = testsig::seeded_noise(AudioSpec::new(22_050, 1), 1, 0.3, 60.0).data;
    let mut group = c.benchmark_group("primitives");
    group.throughput(Throughput::Elements(mono.len() as u64));
    group.sample_size(10);
    group.bench_function("kick band + onsets 60 s @ 22.05 kHz", |b| {
        b.iter(|| {
            let mut band = mono.clone();
            KickBand::new(22_050).process_block(&mut band);
            OnsetDetector::new(22_050).detect(std::hint::black_box(&band))
        });
    });
    let full = testsig::seeded_noise(AudioSpec::new(44_100, 1), 1, 0.3, 60.0).data;
    group.bench_function("resample 60 s 44.1k -> 22.05k", |b| {
        b.iter(|| resample_all(std::hint::black_box(&full), 44_100, 22_050).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_primitives);
criterion_main!(benches);
