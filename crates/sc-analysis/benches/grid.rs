#![allow(
    missing_docs,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // benchmark fixture arithmetic on small counts
//! Grid solving and refitting on a six-minute track at 128 BPM (768 beats, one kick per beat,
//! hi-hat attacks on the eighths). Budgets: a full solve well under 20 ms, a refit (tempo and bar
//! 1 pinned by the user) under 5 ms.

use criterion::{Criterion, criterion_group, criterion_main};
use sc_analysis::grid::{Evidence, SolveSettings, TimedOnset, fit_beats, solve};
use sc_analysis::meter::estimate;

/// Beats, downbeats, activations, kick onsets, broadband onsets.
type Evidence5 = (
    Vec<f64>,
    Vec<f64>,
    Vec<f32>,
    Vec<TimedOnset>,
    Vec<TimedOnset>,
);

fn evidence() -> Evidence5 {
    let period = 60.0 / 128.0;
    let n = 768;
    let beats: Vec<f64> = (0..n)
        .map(|i| ((0.5 + period * i as f64) * 50.0).round() / 50.0)
        .collect();
    let downbeats: Vec<f64> = beats.iter().step_by(4).copied().collect();
    let mut logits = vec![-6.0_f32; 370 * 50];
    for i in 0..n {
        let f = ((0.5 + period * i as f64) * 50.0).round() as usize;
        logits[f] = if i % 4 == 0 { 3.0 } else { -3.0 };
    }
    let kick: Vec<TimedOnset> = (0..n)
        .map(|i| TimedOnset {
            time_s: 0.5 + period * i as f64,
            rise_db: 30.0,
            level_db: if i % 4 == 0 { 0.0 } else { -3.0 },
        })
        .collect();
    let broadband: Vec<TimedOnset> = (0..2 * n)
        .map(|i| TimedOnset {
            time_s: 0.5 + period / 2.0 * i as f64,
            rise_db: 20.0,
            level_db: if i % 8 == 0 { 0.0 } else { -8.0 },
        })
        .collect();
    (beats, downbeats, logits, kick, broadband)
}

fn bench_grid(c: &mut Criterion) {
    let (beats, downbeats, logits, kick, broadband) = evidence();
    let ev = Evidence {
        beats_s: &beats,
        downbeats_s: &downbeats,
        downbeat_logits: &logits,
        kick_onsets: &kick,
        broadband_onsets: &broadband,
    };
    let settings = SolveSettings::default();
    let pinned = SolveSettings {
        bpm_override: Some(128.0),
        anchor_override_s: Some(0.5),
        ..SolveSettings::default()
    };
    let mut group = c.benchmark_group("grid");
    group.bench_function("solve 768 beats", |b| {
        b.iter(|| solve(std::hint::black_box(&ev), &settings, 44_100).unwrap());
    });
    group.bench_function("refit 768 beats (tempo + bar 1 pinned)", |b| {
        b.iter(|| solve(std::hint::black_box(&ev), &pinned, 44_100).unwrap());
    });
    group.bench_function("meter estimate 768 beats", |b| {
        let fit = fit_beats(&beats).unwrap();
        b.iter(|| {
            estimate(
                fit.period,
                fit.phase,
                fit.span,
                std::hint::black_box(&kick),
                &broadband,
                &logits,
                "",
            )
        });
    });
    group.finish();
}

criterion_group!(benches, bench_grid);
criterion_main!(benches);
