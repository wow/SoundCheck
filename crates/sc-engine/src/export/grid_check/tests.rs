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
        detector_offset_ms: None,
        detector_bpm: None,
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
    let check = compare(&e, Some(&grid(220, 120.0, Meter::four_four())), RATE);
    assert_eq!(
        check,
        GridCheck::Pass {
            offset_ms: 0.0,
            bpm_diff: 0.0,
        }
    );
    // A bar later is the same downbeat; 4.9 ms either way still passes, 5.1 ms does not.
    let later = compare(
        &e,
        Some(&grid(220 + 88_200, 120.0, Meter::four_four())),
        RATE,
    );
    assert!(later.passed() && offset(&later).abs() < 1e-9, "{later:?}");
    for (shift, pass) in [(216_i64, true), (-216, true), (225, false), (-225, false)] {
        let anchor = u64::try_from(220 + 88_200 + shift).expect("positive");
        let check = compare(&e, Some(&grid(anchor, 120.0, Meter::four_four())), RATE);
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
    );
    assert!(matches!(check, GridCheck::OffBy { .. }), "{check:?}");
    assert!((offset(&check) + 20.0).abs() < 1e-6);
    assert_eq!(check.to_string(), "off by -20.0 ms (BPM +0.0000)");
}

#[test]
fn another_beat_is_off_by_a_beat() {
    let e = exported(Meter::four_four());
    let beat_2 = grid(220 + 22_050, 120.0, Meter::four_four());
    let check = compare(&e, Some(&beat_2), RATE);
    assert!((offset(&check) - 500.0).abs() < 1e-6, "{check:?}");
}

/// `exported` as the user edited it, with what the source's detector found: its bar line
/// `detector_ms` from the exported bar 1, at `detector_bpm`.
fn edited(mut e: ExportedGrid, detector_ms: f64, detector_bpm: f64) -> ExportedGrid {
    e.edited = true;
    e.confirmed = true;
    e.detector_offset_ms = Some(detector_ms);
    e.detector_bpm = Some(Bpm(detector_bpm));
    e
}

#[test]
fn the_users_own_choices_are_neutral() {
    // The detector of the written file puts its bar line at the lead (220), at 120.000 BPM,
    // exactly where the source's detector put it.
    let found = grid(220, 120.0, Meter::four_four());
    // Bar 1 nudged 12 ms (529 samples) later than the detector's line.
    let mut nudged = exported(Meter::four_four());
    nudged.bar1 = SampleIndex(220 + 529);
    let nudged = edited(nudged, -529.0 * 1000.0 / f64::from(RATE), 120.0);
    let check = compare(&nudged, Some(&found), RATE);
    assert!(check.passed() && offset(&check).abs() < 1e-6, "{check:?}");
    // A typed tempo 0.02 BPM off the fitted one.
    let mut typed = exported(Meter::four_four());
    typed.bpm_exact = Bpm(120.02);
    let check = compare(&edited(typed, 0.0, 120.0), Some(&found), RATE);
    assert!(check.passed(), "{check:?}");
    // Beat 1 moved to the detector's beat 2: the source's detector bar line sits a beat
    // before the exported bar 1.
    let shifted = edited(exported(Meter::four_four()), -500.0, 120.0);
    let check = compare(
        &shifted,
        Some(&grid(220 - 220, 120.0, Meter::four_four())),
        RATE,
    );
    assert!(
        !check.passed(),
        "the detector's line is 5 ms off here: {check:?}"
    );
    let detector_at = 220 + 88_200 - 22_050; // the detector's bar line, a beat before bar 1
    let check = compare(
        &shifted,
        Some(&grid(detector_at, 120.0, Meter::four_four())),
        RATE,
    );
    assert!(check.passed() && offset(&check).abs() < 1e-6, "{check:?}");
}

#[test]
fn a_cut_one_beat_off_fails_even_when_the_user_chose_beat_1() {
    // The audio lost a beat more than recorded: every detector line comes a beat (500 ms)
    // earlier than expected, modulo the bar.
    let shifted = edited(exported(Meter::four_four()), -500.0, 120.0);
    let detector_at = 220 + 88_200 - 22_050 - 22_050;
    let check = compare(
        &shifted,
        Some(&grid(detector_at, 120.0, Meter::four_four())),
        RATE,
    );
    assert!(matches!(check, GridCheck::OffBy { .. }), "{check:?}");
    assert!((offset(&check) + 500.0).abs() < 1e-6, "{check:?}");
    let plain = compare(
        &exported(Meter::four_four()),
        Some(&grid(220 + 88_200 - 22_050, 120.0, Meter::four_four())),
        RATE,
    );
    assert!((offset(&plain) + 500.0).abs() < 1e-6, "{plain:?}");
}

#[test]
fn tempo_meter_and_missing_grids_fail() {
    let e = exported(Meter::four_four());
    let slow = compare(&e, Some(&grid(220, 120.006, Meter::four_four())), RATE);
    assert!(
        matches!(slow, GridCheck::BpmDiffers { bpm_diff, found } if (bpm_diff - 0.006).abs() < 1e-9 && found == Bpm(120.006)),
        "{slow:?}"
    );
    let within = compare(&e, Some(&grid(220, 119.995, Meter::four_four())), RATE);
    assert!(within.passed(), "{within:?}");
    let double = compare(&e, Some(&grid(220, 240.0, Meter::four_four())), RATE);
    assert!(matches!(double, GridCheck::BpmDiffers { .. }));
    let waltz = compare(&e, Some(&grid(220, 120.0, Meter::three_four())), RATE);
    assert_eq!(
        waltz,
        GridCheck::MeterDiffers {
            found: Meter::three_four()
        }
    );
    assert_eq!(compare(&e, None, RATE), GridCheck::NoGridFound);
}

#[test]
fn regular_meters_are_checked_and_odd_ones_are_not() {
    for meter in [Meter::three_four(), Meter::six_eight()] {
        let check = compare(
            &exported(meter.clone()),
            Some(&grid(220, 120.0, meter.clone())),
            RATE,
        );
        assert!(check.passed(), "{meter}: {check:?}");
    }
    let aksak = Meter::nine_eight_aksak();
    assert_eq!(
        compare(
            &exported(aksak.clone()),
            Some(&grid(220, 120.0, aksak.clone())),
            RATE
        ),
        GridCheck::NotChecked {
            reason: GridCheckSkip::OddMeter { meter: aksak }
        }
    );
}
