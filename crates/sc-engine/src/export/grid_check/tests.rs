//! Unit tests of `crates/sc-engine/src/export/grid_check.rs`: the comparison on hand-made grids.
//! At 44.1 kHz and 120 BPM a beat is 22,050 samples and a 4/4 bar 88,200; 5 ms is 220.5 samples.
use super::*;
use sc_core::analysis::{Alternatives, Confidence, Meter, Verdict};
use sc_core::{Bpm, SampleIndex};

const RATE: u32 = 44_100;

fn grid(anchor: u64, bpm: f64, meter: Meter) -> Grid {
    Grid {
        anchor: SampleIndex(anchor),
        bpm: Bpm(bpm),
        meter,
        meter_runner_up: None,
        first_downbeat_index: 0,
        phrase_len_bars: 8,
        segments: Vec::new(),
        residual_p95_ms: 2.0,
        residual_max_ms: 4.0,
        local_bpm_range: 0.0,
        drift_ppm: 0.0,
        verdict: Verdict::Static,
        confidence: Confidence::Green,
        reasons: Vec::new(),
        alternatives: Alternatives::default(),
    }
}

/// An exported 120 BPM 4/4 grid with bar 1 at the 5 ms lead (220 samples).
fn exported(meter: Meter) -> ExportedGrid {
    ExportedGrid {
        bar1: SampleIndex(220),
        first_bar_line: SampleIndex(220),
        bpm: Bpm(120.0),
        bpm_exact: Bpm(120.0),
        meter,
        edited: false,
        confirmed: false,
    }
}

fn offset(check: &GridCheck) -> f64 {
    match check {
        GridCheck::Pass { offset_ms, .. } | GridCheck::OffBy { offset_ms, .. } => *offset_ms,
        other => panic!("no offset: {other:?}"),
    }
}

#[test]
fn bar_1_at_the_lead_passes() {
    let e = exported(Meter::four_four());
    let check = compare(
        &e,
        Some(&grid(220, 120.0, Meter::four_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert_eq!(
        check,
        GridCheck::Pass {
            offset_ms: 0.0,
            bpm_diff: 0.0,
            period: CheckPeriod::Bar
        }
    );
    // A bar later is the same downbeat; 4.9 ms either way still passes, 5.1 ms does not.
    let later = compare(
        &e,
        Some(&grid(220 + 88_200, 120.0, Meter::four_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert!(later.passed() && offset(&later).abs() < 1e-9, "{later:?}");
    for (shift, pass) in [(216_i64, true), (-216, true), (225, false), (-225, false)] {
        let anchor = u64::try_from(220 + 88_200 + shift).expect("positive");
        let check = compare(
            &e,
            Some(&grid(anchor, 120.0, Meter::four_four())),
            RATE,
            CheckPeriod::Bar,
        );
        assert_eq!(check.passed(), pass, "{shift}: {check:?}");
        #[allow(clippy::cast_precision_loss)]
        let expected = shift as f64 * 1000.0 / f64::from(RATE);
        assert!((offset(&check) - expected).abs() < 1e-6, "{check:?}");
    }
}

#[test]
fn an_extra_20_ms_cut_is_off_by_20_ms() {
    // The audio lost 20 ms more than the export recorded: the detector's bar 1 sits 20 ms
    // (882 samples) before the exported one.
    let check = compare(
        &exported(Meter::four_four()),
        Some(&grid(88_200 + 220 - 882, 120.0, Meter::four_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert!(matches!(check, GridCheck::OffBy { .. }), "{check:?}");
    assert!((offset(&check) + 20.0).abs() < 1e-6);
    assert_eq!(check.to_string(), "off by -20.0 ms (bar line, BPM +0.0000)");
}

#[test]
fn another_beat_is_off_modulo_the_bar_but_not_the_beat() {
    let e = exported(Meter::four_four());
    let beat_2 = grid(220 + 22_050, 120.0, Meter::four_four());
    let bar = compare(&e, Some(&beat_2), RATE, CheckPeriod::Bar);
    assert!((offset(&bar) - 500.0).abs() < 1e-6, "{bar:?}");
    let beat = compare(&e, Some(&beat_2), RATE, CheckPeriod::Beat);
    assert!(beat.passed(), "{beat:?}");
}

#[test]
fn tempo_meter_and_missing_grids_fail() {
    let e = exported(Meter::four_four());
    let slow = compare(
        &e,
        Some(&grid(220, 120.006, Meter::four_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert!(
        matches!(slow, GridCheck::BpmDiffers { bpm_diff, found } if (bpm_diff - 0.006).abs() < 1e-9 && found == Bpm(120.006)),
        "{slow:?}"
    );
    let within = compare(
        &e,
        Some(&grid(220, 119.995, Meter::four_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert!(within.passed(), "{within:?}");
    let double = compare(
        &e,
        Some(&grid(220, 240.0, Meter::four_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert!(matches!(double, GridCheck::BpmDiffers { .. }));
    let waltz = compare(
        &e,
        Some(&grid(220, 120.0, Meter::three_four())),
        RATE,
        CheckPeriod::Bar,
    );
    assert_eq!(
        waltz,
        GridCheck::MeterDiffers {
            found: Meter::three_four()
        }
    );
    assert_eq!(
        compare(&e, None, RATE, CheckPeriod::Bar),
        GridCheck::NoGridFound
    );
}

#[test]
fn regular_meters_are_checked_and_odd_ones_are_not() {
    for meter in [Meter::three_four(), Meter::six_eight()] {
        let check = compare(
            &exported(meter.clone()),
            Some(&grid(220, 120.0, meter.clone())),
            RATE,
            CheckPeriod::Bar,
        );
        assert!(check.passed(), "{meter}: {check:?}");
    }
    let aksak = Meter::nine_eight_aksak();
    assert_eq!(
        compare(
            &exported(aksak.clone()),
            Some(&grid(220, 120.0, aksak.clone())),
            RATE,
            CheckPeriod::Bar
        ),
        GridCheck::NotChecked {
            reason: GridCheckSkip::OddMeter { meter: aksak }
        }
    );
}
