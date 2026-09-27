//! Unit tests of the private parts of `crates/sc-core/src/analysis/mod.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;

#[test]
fn settings_hash_is_stable_and_sensitive() {
    let a = AnalysisSettings::default();
    assert_eq!(a.settings_hash(), a.clone().settings_hash());
    assert_eq!(
        a.settings_hash(),
        0x4d9f_9a82_8cbc_1c2a,
        "canonical text changed"
    );
    let b = AnalysisSettings {
        grid: false,
        ..AnalysisSettings::default()
    };
    let c = AnalysisSettings {
        bpm_range: (Bpm(80.0), Bpm(180.0)),
        ..AnalysisSettings::default()
    };
    assert_ne!(a.settings_hash(), b.settings_hash());
    assert_ne!(a.settings_hash(), c.settings_hash());
}

#[test]
fn fnv1a_matches_the_reference_vectors() {
    assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
}

#[test]
fn record_serialises_in_camel_case_with_units_flattened() {
    let record = AnalysisRecord {
        schema: RECORD_SCHEMA,
        version: crate::VERSION.into(),
        path: "/music/track.flac".into(),
        size: 1,
        mtime_ns: 2,
        spec: AudioSpec::CD,
        frames: 44_100,
        duration: Seconds(1.0),
        delay: 0,
        padding: 0,
        loudness: LoudnessReport {
            integrated: Some(Lufs(-11.0)),
            momentary_max: None,
            short_term_max: None,
            short_term_p95: Some(Lufs(-9.5)),
            short_term_top30: None,
            lra: Some(Lu(4.0)),
            true_peak: DbTp(-0.3),
            sample_peak: DbFs(-0.5),
            plr: Some(Lu(10.7)),
            dual_mono: false,
            timeline: Timeline {
                hop_ms: TIMELINE_HOP_MS,
                short_term: vec![None, Some(-12.0)],
            },
        },
        grid: None,
        grid_skipped: Some("not requested".into()),
        tags: TagHints::default(),
        evidence: None,
    };
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("\"shortTermP95\":-9.5"), "{json}");
    assert!(json.contains("\"shortTerm\":[null,-12.0]"), "{json}");
    assert!(json.contains("\"gridSkipped\":\"not requested\""), "{json}");
    let back: AnalysisRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, record);
}
