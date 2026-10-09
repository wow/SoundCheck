#![allow(missing_docs)] // criterion_group! generates undocumented items
//! The player's meter on six minutes of stereo 44.1 kHz audio, fed in player blocks of 1,024
//! frames as the feeding thread does: true peak (4x oversampled) and momentary loudness per
//! block. Budget: at least 50 times real time (2 % of one core while playing).

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use sc_core::{AudioSpec, SampleIndex, testsig};
use sc_engine::player::{BLOCK_FRAMES, Meters, Metering};

const SECONDS: f64 = 360.0;

fn meter(c: &mut Criterion) {
    let audio = testsig::seeded_noise(AudioSpec::CD, 5, 0.5, SECONDS);
    let meters = Arc::new(Meters::new());
    let mut group = c.benchmark_group("meter");
    group.sample_size(10);
    group.bench_function("six_minutes_stereo_44k1", |b| {
        b.iter(|| {
            let mut metering = Metering::new(44_100, 2, Arc::clone(&meters)).expect("a meter");
            let mut end = 0;
            for block in audio.data.chunks(BLOCK_FRAMES * 2) {
                end += (block.len() / 2) as u64;
                metering.push(block, SampleIndex(end));
            }
            meters.latest()
        });
    });
    group.finish();
}

criterion_group!(benches, meter);
criterion_main!(benches);
