//! Unit tests of the private parts of `crates/sc-core/src/analysis/edit.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;
use crate::analysis::{AnalysisRecord, BeatUnit};

fn record_with(evidence: Option<GridEvidence>) -> AnalysisRecord {
    let json = r#"{"schema":2,"version":"0","path":"/a.wav","size":1,"mtimeNs":2,
        "spec":{"sampleRate":44100,"channels":2},"frames":1,"duration":1.0,"delay":0,
        "padding":0,"loudness":{"integrated":null,"momentaryMax":null,"shortTermMax":null,
        "shortTermP95":null,"shortTermTop30":null,"lra":null,"truePeak":-1.0,
        "samplePeak":-1.0,"plr":null,"dualMono":false,"timeline":{"hopMs":100,
        "shortTerm":[]}},"grid":null,"gridSkipped":null,"tags":{"bpm":null,"genre":null,
        "title":null,"artist":null},"evidence":null}"#;
    let mut record: AnalysisRecord = serde_json::from_str(json).unwrap();
    record.evidence = evidence;
    record
}

#[test]
fn evidence_round_trips_exactly_through_json() {
    let onsets = OnsetList {
        sample_rate: 22_050,
        frames: vec![0, 11_025, u32::MAX],
        rise_db: vec![6.123_456_7, 30.0, f32::MIN_POSITIVE],
        level_db: vec![-80.0, -0.000_123_4, -12.345_678],
    };
    let evidence = GridEvidence {
        beats_s: vec![0.1, 0.52, 1.0 / 3.0, 123.456_79],
        downbeats_s: vec![0.52],
        downbeat_logits_50fps: vec![-3.256_789_4, 0.0, 2.123_456_7],
        kick_onsets: onsets.clone(),
        broadband_onsets: onsets,
    };
    let record = record_with(Some(evidence));
    let json = serde_json::to_string(&record).unwrap();
    let back: AnalysisRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back.evidence, record.evidence, "every f32 bit survives");
}

#[test]
fn grid_edits_are_checked_against_their_limits() {
    assert!(GridEdit::default().validate().is_ok());
    let ok = GridEdit {
        bpm: Some(Bpm(399.0)),
        octave: -3,
        meter: Some(Meter::ten_eight()),
        ..GridEdit::default()
    };
    assert!(ok.validate().is_ok());
    for bad in [
        GridEdit {
            bpm: Some(Bpm(f64::INFINITY)),
            ..GridEdit::default()
        },
        GridEdit {
            tempo_hint: Some(Bpm(1000.5)),
            ..GridEdit::default()
        },
        GridEdit {
            octave: 4,
            ..GridEdit::default()
        },
        GridEdit {
            meter: Some(Meter {
                beats_per_bar: 0,
                unit: BeatUnit::Quarter,
                grouping: vec![],
            }),
            ..GridEdit::default()
        },
        GridEdit {
            meter: Some(Meter {
                beats_per_bar: 4,
                unit: BeatUnit::Quarter,
                grouping: vec![1, 0, 3],
            }),
            ..GridEdit::default()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn a_grid_edit_defaults_to_no_edit_and_reads_missing_fields() {
    assert!(GridEdit::default().is_empty());
    let edit: GridEdit = serde_json::from_str(r#"{"octave":1}"#).unwrap();
    assert_eq!(edit.octave, 1);
    assert!(!edit.is_empty());
    assert_eq!(edit.anchor, None);
    let json = serde_json::to_string(&GridEdit {
        meter: Some(Meter::nine_eight_aksak()),
        anchor: Some(SampleIndex(44_100)),
        ..GridEdit::default()
    })
    .unwrap();
    assert!(json.contains("\"tempoHint\":null"), "{json}");
    assert!(json.contains("\"downbeatShift\":0"), "{json}");
}

#[test]
fn a_saved_edit_without_a_fit_reads_as_the_whole_track_fit() {
    let edit: GridEdit = serde_json::from_str(r#"{"octave":0,"downbeatShift":0}"#).unwrap();
    assert_eq!(edit.fit, GridFit::Whole);
    assert!(edit.is_empty(), "the whole-track fit is no edit");
    assert_eq!(GridEdit::default().fit, GridFit::Whole);
}

#[test]
fn a_start_fit_round_trips_through_json() {
    let edit: GridEdit = serde_json::from_str(r#"{"fit":"start"}"#).unwrap();
    assert_eq!(edit.fit, GridFit::Start);
    assert!(!edit.is_empty());
    let json = serde_json::to_string(&edit).unwrap();
    assert!(json.contains("\"fit\":\"start\""), "{json}");
    assert_eq!(serde_json::from_str::<GridEdit>(&json).unwrap(), edit);
    let whole = serde_json::to_string(&GridEdit::default()).unwrap();
    assert!(whole.contains("\"fit\":\"whole\""), "{whole}");
}
