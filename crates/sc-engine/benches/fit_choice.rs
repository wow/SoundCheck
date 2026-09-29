#![allow(missing_docs, clippy::cast_possible_truncation, clippy::cast_sign_loss)] // criterion_group! generates undocumented items; fixture arithmetic
//! What the grid view runs on every refit of a track whose tempo changes: the edit's refit plus
//! the fit choice (the other fit refitted, the start window chosen, both shares), on a six-minute
//! track at 116 BPM for a minute and 117.2 after (about 700 beats, a kick on each). Budget: the
//! refit and the choice together under 5 ms.

use criterion::{Criterion, criterion_group, criterion_main};
use sc_core::Bpm;
use sc_core::analysis::{AnalysisRecord, GridEdit, GridEvidence, GridFit, OnsetList};
use sc_engine::edits::refit_record;
use sc_engine::fit_choice;

const RANGE: (Bpm, Bpm) = (Bpm(70.0), Bpm(180.0));

fn evidence() -> GridEvidence {
    let mut times = Vec::new();
    let mut t: f64 = 0.5;
    while t < 360.0 {
        times.push(t);
        t += 60.0 / if t < 60.0 { 116.0 } else { 117.2 };
    }
    let kicks = OnsetList {
        sample_rate: 22_050,
        frames: times
            .iter()
            .map(|t| ((t * 1000.0).round() / 1000.0 * 22_050.0).round() as u32)
            .collect(),
        rise_db: vec![30.0; times.len()],
        level_db: (0..times.len())
            .map(|i| if i % 4 == 0 { 0.0 } else { -3.0 })
            .collect(),
    };
    let mut logits = vec![-6.0_f32; 360 * 50 + 2];
    let (mut beats_s, mut downbeats_s) = (Vec::new(), Vec::new());
    for (i, t) in times.iter().enumerate() {
        let frame = (t * 50.0).round();
        beats_s.push((frame / 50.0) as f32);
        if i % 4 == 0 {
            downbeats_s.push((frame / 50.0) as f32);
            logits[frame as usize] = 4.0;
        }
    }
    GridEvidence {
        beats_s,
        downbeats_s,
        downbeat_logits_50fps: logits,
        kick_onsets: kicks.clone(),
        broadband_onsets: kicks,
    }
}

fn record() -> AnalysisRecord {
    let json = r#"{"schema":2,"version":"0","path":"/a.wav","size":1,"mtimeNs":2,
        "spec":{"sampleRate":44100,"channels":2},"frames":15876000,"duration":360.0,"delay":0,
        "padding":0,"loudness":{"integrated":null,"momentaryMax":null,"shortTermMax":null,
        "shortTermP95":null,"shortTermTop30":null,"lra":null,"truePeak":-1.0,
        "samplePeak":-1.0,"plr":null,"dualMono":false,"timeline":{"hopMs":100,
        "shortTerm":[]}},"grid":null,"gridSkipped":null,"tags":{"bpm":null,"genre":null,
        "title":null,"artist":null},"evidence":null}"#;
    let mut record: AnalysisRecord = serde_json::from_str(json).expect("a record");
    record.evidence = Some(evidence());
    record
}

fn bench_fit_choice(c: &mut Criterion) {
    let record = record();
    let mut group = c.benchmark_group("fit choice");
    for fit in [GridFit::Whole, GridFit::Start] {
        let edit = GridEdit {
            fit,
            ..GridEdit::default()
        };
        group.bench_function(
            format!("refit + fit choice, two tempi ({fit:?} edit)"),
            |b| {
                b.iter(|| {
                    let solved = refit_record(std::hint::black_box(&record), RANGE, &edit);
                    fit_choice(&record, RANGE, &edit, solved.as_ref())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_fit_choice);
criterion_main!(benches);
