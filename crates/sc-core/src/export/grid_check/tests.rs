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
            found: Meter::three_four()
        }
        .to_string(),
        "meter differs (found 3/4)"
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
