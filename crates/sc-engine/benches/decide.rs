#![allow(missing_docs)] // criterion_group! generates undocumented items
//! Replanning a whole table: DECIDE over 1,000 analysed rows, as happens when the user switches
//! DJ | Streaming or edits the target. Budget: well under 10 ms.

use criterion::{Criterion, criterion_group, criterion_main};
use sc_core::analysis::{
    Alternatives, AnalysisRecord, Grid, LoudnessReport, Meter, RECORD_SCHEMA, TagHints, Timeline,
};
use sc_core::plan::{Codec, DecideSettings};
use sc_core::{AudioSpec, Bpm, Confidence, DbFs, DbTp, Lufs, SampleIndex, Seconds, Verdict};
use sc_engine::decide;

fn record(i: u32) -> AnalysisRecord {
    let level = -6.0 - f64::from(i % 120) / 10.0;
    AnalysisRecord {
        schema: RECORD_SCHEMA,
        version: "bench".into(),
        path: format!("/music/{i}.flac"),
        size: 1,
        mtime_ns: 1,
        spec: AudioSpec::CD,
        frames: 44_100 * 240,
        duration: Seconds(240.0),
        delay: 0,
        padding: 0,
        loudness: LoudnessReport {
            integrated: Some(Lufs(level - 1.5)),
            momentary_max: None,
            short_term_max: None,
            short_term_p95: Some(Lufs(level)),
            short_term_top30: None,
            lra: None,
            true_peak: DbTp(-0.1 - f64::from(i % 30) / 10.0),
            sample_peak: DbFs(-0.2),
            plr: None,
            dual_mono: false,
            timeline: Timeline {
                hop_ms: 100,
                short_term: Vec::new(),
            },
        },
        grid: Some(Grid {
            anchor: SampleIndex(0),
            bpm: Bpm(90.0 + f64::from(i % 90)),
            meter: Meter::four_four(),
            meter_runner_up: None,
            first_downbeat_index: 0,
            phrase_len_bars: 8,
            segments: Vec::new(),
            residual_p95_ms: 5.0,
            residual_max_ms: 10.0,
            local_bpm_range: 0.0,
            drift_ppm: 0.0,
            verdict: if i.is_multiple_of(7) {
                Verdict::Drifts
            } else {
                Verdict::Static
            },
            confidence: if i.is_multiple_of(5) {
                Confidence::Amber
            } else {
                Confidence::Green
            },
            reasons: Vec::new(),
            alternatives: Alternatives::default(),
        }),
        grid_skipped: None,
        tags: TagHints {
            bpm: Some(Bpm(128.0)),
            ..TagHints::default()
        },
        evidence: None,
    }
}

fn replan(c: &mut Criterion) {
    let rows: Vec<(AnalysisRecord, Codec)> = (0..1000)
        .map(|i| {
            (
                record(i),
                if i.is_multiple_of(4) {
                    Codec::Mp3
                } else {
                    Codec::Flac
                },
            )
        })
        .collect();
    let settings = DecideSettings::dj();
    c.bench_function("decide 1000 rows", |b| {
        b.iter(|| {
            rows.iter()
                .map(|(r, codec)| decide(r, *codec, &settings))
                .filter(|p| !p.review.is_empty())
                .count()
        });
    });
}

criterion_group!(benches, replan);
criterion_main!(benches);
