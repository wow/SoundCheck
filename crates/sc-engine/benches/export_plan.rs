#![allow(missing_docs)] // criterion_group! generates undocumented items
//! Planning the export of a whole table: DECIDE plus PLAN EXPORT over 1,000 analysed rows, as
//! the confirm summary does before an export. Budget: well under 10 ms.

use criterion::{Criterion, criterion_group, criterion_main};
use sc_core::analysis::{
    Alternatives, AnalysisRecord, Grid, LoudnessReport, Meter, RECORD_SCHEMA, TagHints, Timeline,
};
use sc_core::export::{BatchMode, ExportOutcome, ExportSettings};
use sc_core::plan::{Codec, DecideSettings};
use sc_core::{AudioSpec, Bpm, Confidence, DbFs, DbTp, Lu, Lufs, SampleIndex, Seconds, Verdict};
use sc_engine::{ExportInput, ExportSource, decide, plan_export};

fn record(i: u32) -> AnalysisRecord {
    let level = -6.0 - f64::from(i % 120) / 10.0;
    AnalysisRecord {
        schema: RECORD_SCHEMA,
        version: "bench".into(),
        path: format!("/music/{i}.wav"),
        size: 1,
        mtime_ns: 1,
        spec: AudioSpec::CD,
        frames: 44_100 * 240,
        duration: Seconds(240.0),
        delay: 0,
        padding: 0,
        loudness: LoudnessReport {
            integrated: Some(Lufs(level - 1.5)),
            momentary_max: Some(Lufs(level + 3.0)),
            short_term_max: Some(Lufs(level + 1.0)),
            short_term_p95: Some(Lufs(level)),
            short_term_top30: None,
            lra: Some(Lu(5.0)),
            true_peak: DbTp(-0.1 - f64::from(i % 30) / 10.0),
            sample_peak: DbFs(-0.2),
            plr: None,
            dual_mono: false,
            timeline: Timeline::default(),
        },
        grid: Some(Grid {
            anchor: SampleIndex(u64::from(i) * 997 % 400_000),
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
            verdict: Verdict::Static,
            confidence: Confidence::Green,
            reasons: Vec::new(),
            alternatives: Alternatives::default(),
        }),
        grid_skipped: None,
        tags: TagHints::default(),
        evidence: None,
    }
}

fn plan_table(c: &mut Criterion) {
    let rows: Vec<(AnalysisRecord, ExportSource)> = (0..1000)
        .map(|i| {
            let codec = match i % 4 {
                0 => Codec::Mp3,
                1 => Codec::Flac,
                2 => Codec::Aiff,
                _ => Codec::Wav,
            };
            let source = ExportSource {
                codec,
                bits_per_sample: Some(24),
                has_tag: true,
                ..ExportSource::default()
            };
            (record(i), source)
        })
        .collect();
    let decide_settings = DecideSettings::dj();
    let settings = ExportSettings::new(BatchMode::Prepare);
    c.bench_function("decide + plan export 1000 rows", |b| {
        b.iter(|| {
            rows.iter()
                .map(|(record, source)| {
                    let plan = decide(record, source.codec, &decide_settings, false);
                    let input = ExportInput {
                        record,
                        plan: &plan,
                        decide: &decide_settings,
                        source,
                    };
                    plan_export(&input, &settings)
                })
                .filter(|o| matches!(o, ExportOutcome::Write { .. }))
                .count()
        });
    });
}

criterion_group!(benches, plan_table);
criterion_main!(benches);
