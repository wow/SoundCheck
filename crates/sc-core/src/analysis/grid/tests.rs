//! Unit tests of the private parts of `crates/sc-core/src/analysis/grid.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;

#[test]
fn meter_templates_are_consistent_and_labelled() {
    for m in Meter::templates() {
        let sum: u32 = m.grouping.iter().map(|&g| u32::from(g)).sum();
        assert_eq!(u32::from(m.beats_per_bar), sum, "{m}");
    }
    assert_eq!(Meter::four_four().to_string(), "4/4");
    assert_eq!(Meter::three_four().to_string(), "3/4");
    assert_eq!(Meter::six_eight().to_string(), "6/8 · 3+3");
    assert_eq!(Meter::nine_eight_aksak().to_string(), "9/8 · 2+2+2+3");
    assert_eq!(Meter::seven_eight().to_string(), "7/8 · 2+2+3");
    assert!(Meter::four_four().is_regular());
    assert!(Meter::six_eight().is_regular());
    assert!(!Meter::nine_eight_aksak().is_regular());
    assert_eq!(Meter::nine_eight_aksak().beats(), 4);
}

#[test]
fn grid_geometry() {
    let grid = Grid {
        anchor: SampleIndex(22_050),
        bpm: Bpm(128.0),
        meter: Meter::four_four(),
        meter_runner_up: None,
        first_downbeat_index: 0,
        phrase_len_bars: 8,
        segments: vec![],
        residual_p95_ms: 3.0,
        residual_max_ms: 9.0,
        local_bpm_range: 0.01,
        drift_ppm: 4.0,
        verdict: Verdict::Static,
        confidence: Confidence::Green,
        reasons: vec![],
        alternatives: Alternatives::default(),
    };
    assert_eq!(grid.samples_per_beat(44_100), 20_671.875);
}
