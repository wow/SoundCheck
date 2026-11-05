//! Unit tests of `crates/sc-core/src/export/grid_check.rs`.
use super::*;

#[test]
fn wording() {
    let pass = GridCheck::Pass {
        offset_ms: -0.04,
        bpm_diff: 0.000_31,
    };
    assert_eq!(pass.to_string(), "pass (bar line +0.0 ms, BPM +0.0003)");
    assert!(pass.passed() && !pass.failed());
    let off = GridCheck::OffBy {
        offset_ms: -20.26,
        bpm_diff: 0.0,
    };
    assert_eq!(off.to_string(), "off by -20.3 ms (BPM +0.0000)");
    assert!(off.failed());
    let bpm = GridCheck::BpmDiffers {
        bpm_diff: 0.012,
        found: Bpm(128.012),
    };
    assert_eq!(bpm.to_string(), "BPM differs by +0.0120 (found 128.012)");
    assert!(bpm.failed());
    assert_eq!(
        GridCheck::MeterDiffers {
            found: Meter::three_four(),
            exported: Some(Meter::four_four()),
        }
        .to_string(),
        "meter differs (found 3/4, exported 4/4)"
    );
    assert_eq!(
        GridCheck::NoGridFound.to_string(),
        "no grid found in the written file"
    );
    let skipped = GridCheck::NotChecked {
        reason: GridCheckSkip::OddMeter {
            meter: Meter::nine_eight_aksak(),
        },
    };
    assert_eq!(skipped.to_string(), "not checked: meter 9/8 · 2+2+2+3");
    assert!(!skipped.passed() && !skipped.failed());
    for (reason, text) in [
        (GridCheckSkip::NoGrid, "not checked: no grid exported"),
        (
            GridCheckSkip::GridWithheld,
            "not checked: grid withheld (needs review)",
        ),
        (
            GridCheckSkip::XmlOnly,
            "not checked: file not written (XML only)",
        ),
        (
            GridCheckSkip::NotAnalysed,
            "not checked: the written file was not analysed",
        ),
    ] {
        assert_eq!(GridCheck::NotChecked { reason }.to_string(), text);
    }
}

#[test]
fn json_shape() {
    let check = GridCheck::Pass {
        offset_ms: 0.5,
        bpm_diff: 0.0,
    };
    let json = serde_json::to_string(&check).expect("serialises");
    assert_eq!(json, r#"{"result":"pass","offsetMs":0.5,"bpmDiff":0.0}"#);
    let skipped = GridCheck::NotChecked {
        reason: GridCheckSkip::XmlOnly,
    };
    let json = serde_json::to_string(&skipped).expect("serialises");
    assert_eq!(
        json,
        r#"{"result":"notChecked","reason":{"type":"xmlOnly"}}"#
    );
    assert_eq!(
        serde_json::from_str::<GridCheck>(&json).expect("reads"),
        skipped
    );
}

fn off(offset_ms: f64) -> GridCheck {
    GridCheck::OffBy {
        offset_ms,
        bpm_diff: 0.0,
    }
}

fn one_beat_earlier() -> GridCheck {
    GridCheck::CheckBar1 {
        offset_ms: -416.74,
        bpm_diff: 0.000_2,
        beats: -1,
    }
}

#[test]
fn a_disagreement_on_a_confirmed_grid_is_advisory_and_keeps_its_numbers() {
    for (check, detail) in [
        (off(-20.26), "off by -20.3 ms (BPM +0.0000)"),
        (one_beat_earlier(), "off by -416.7 ms (BPM +0.0002)"),
        (
            GridCheck::BpmDiffers {
                bpm_diff: 0.012,
                found: Bpm(128.012),
            },
            "BPM differs by +0.0120 (found 128.012)",
        ),
        (
            GridCheck::MeterDiffers {
                found: Meter::three_four(),
                exported: Some(Meter::four_four()),
            },
            "meter differs (found 3/4, exported 4/4)",
        ),
        (GridCheck::NoGridFound, "no grid found in the written file"),
    ] {
        let advisory = check.on_confirmed_grid();
        assert!(
            matches!(advisory, GridCheck::Advisory { .. }),
            "{advisory:?}"
        );
        assert!(!advisory.failed() && !advisory.passed(), "{advisory:?}");
        assert_eq!(
            advisory.to_string(),
            format!("the analysis reads this differently; your confirmed grid was kept: {detail}")
        );
    }
}

#[test]
fn agreement_and_not_checked_stay_as_they_are_on_a_confirmed_grid() {
    let pass = GridCheck::Pass {
        offset_ms: 0.5,
        bpm_diff: 0.0,
    };
    assert_eq!(pass.clone().on_confirmed_grid(), pass);
    let skipped = GridCheck::NotChecked {
        reason: GridCheckSkip::NoGrid,
    };
    assert_eq!(skipped.clone().on_confirmed_grid(), skipped);
    let advisory = off(9.0).on_confirmed_grid();
    assert_eq!(advisory.clone().on_confirmed_grid(), advisory);
}

#[test]
fn check_bar_1_is_neither_a_pass_nor_a_failure_and_says_which_way() {
    let check = one_beat_earlier();
    assert!(!check.passed() && !check.failed());
    assert_eq!(
        check.to_string(),
        "check bar 1: the analysis reads it 1 beat earlier (bar line -416.7 ms, BPM +0.0002)"
    );
    let later = GridCheck::CheckBar1 {
        offset_ms: 1000.0,
        bpm_diff: 0.0,
        beats: 2,
    };
    assert_eq!(
        later.to_string(),
        "check bar 1: the analysis reads it 2 beats later (bar line +1000.0 ms, BPM +0.0000)"
    );
}

#[test]
fn the_outcomes_differ_by_their_result_alone() {
    let advisory = off(-20.0).on_confirmed_grid();
    assert_eq!(
        serde_json::to_string(&advisory).expect("serialises"),
        r#"{"result":"advisory","disagreement":{"kind":"offBy","offsetMs":-20.0,"bpmDiff":0.0}}"#
    );
    let check_bar_1 = GridCheck::CheckBar1 {
        offset_ms: 500.0,
        bpm_diff: 0.0,
        beats: 1,
    };
    assert_eq!(
        serde_json::to_string(&check_bar_1).expect("serialises"),
        r#"{"result":"checkBar1","offsetMs":500.0,"bpmDiff":0.0,"beats":1}"#
    );
    // What older versions wrote keeps its shape.
    assert_eq!(
        serde_json::to_string(&off(-20.0)).expect("serialises"),
        r#"{"result":"offBy","offsetMs":-20.0,"bpmDiff":0.0}"#
    );
    assert_eq!(
        serde_json::to_string(&GridCheck::NoGridFound).expect("serialises"),
        r#"{"result":"noGridFound"}"#
    );
    for check in [
        advisory,
        check_bar_1,
        GridCheck::BpmDiffers {
            bpm_diff: 0.012,
            found: Bpm(128.012),
        }
        .on_confirmed_grid(),
        GridCheck::MeterDiffers {
            found: Meter::three_four(),
            exported: Some(Meter::four_four()),
        }
        .on_confirmed_grid(),
        GridCheck::NoGridFound.on_confirmed_grid(),
    ] {
        let json = serde_json::to_string(&check).expect("serialises");
        assert_eq!(
            serde_json::from_str::<GridCheck>(&json).expect("reads"),
            check,
            "{json}"
        );
    }
}

#[test]
fn a_meter_result_recorded_without_the_exported_meter_still_reads_and_says_what_it_knows() {
    let found = serde_json::to_string(&Meter::three_four()).expect("serialises");
    let json = format!(r#"{{"result":"meterDiffers","found":{found}}}"#);
    let check: GridCheck = serde_json::from_str(&json).expect("reads");
    assert_eq!(
        check,
        GridCheck::MeterDiffers {
            found: Meter::three_four(),
            exported: None,
        }
    );
    assert_eq!(check.to_string(), "meter differs (found 3/4)");
    // Written back, it stays as it was recorded.
    assert_eq!(serde_json::to_string(&check).expect("serialises"), json);
}
