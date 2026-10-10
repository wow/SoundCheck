//! Unit tests of `crates/sc-core/src/export.rs`.
use super::*;

#[test]
fn defaults_per_mode() {
    let prepare = ExportSettings::new(BatchMode::Prepare);
    assert!(prepare.tbpm && prepare.xml && !prepare.grid_only);
    assert_eq!(prepare.place, Place::InPlace);
    assert_eq!(prepare.depth, None);
    assert!((prepare.lead_ms - 5.0).abs() < f64::EPSILON);
    let library = ExportSettings::new(BatchMode::Library);
    assert!(!library.tbpm && library.xml);
    assert_eq!(ExportSettings::default(), prepare);
    assert!(prepare.validate().is_ok() && library.validate().is_ok());
}

#[test]
fn settings_outside_their_limits_are_rejected() {
    let base = ExportSettings::default();
    for depth in [8, 20, 32] {
        let s = ExportSettings {
            depth: Some(depth),
            ..base
        };
        assert!(s.validate().is_err(), "{depth}");
    }
    for depth in [16, 24] {
        let s = ExportSettings {
            depth: Some(depth),
            ..base
        };
        assert!(s.validate().is_ok(), "{depth}");
    }
    for lead in [-1.0, 50.5, f64::NAN, f64::INFINITY] {
        let s = ExportSettings {
            lead_ms: lead,
            ..base
        };
        assert!(s.validate().is_err(), "{lead}");
    }
    let zero = ExportSettings {
        lead_ms: 0.0,
        ..base
    };
    assert!(zero.validate().is_ok());
}

#[test]
fn outcomes_serialise_as_tagged_unions() {
    let skip = ExportOutcome::Skip {
        reason: ExportSkip::NotDjSafeRate {
            sample_rate_hz: 96_000,
        },
    };
    assert_eq!(
        serde_json::to_string(&skip).unwrap(),
        r#"{"type":"skip","reason":{"type":"notDjSafeRate","sampleRateHz":96000}}"#
    );
    let xml = ExportOutcome::XmlOnly {
        reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 },
    };
    assert_eq!(
        serde_json::to_string(&xml).unwrap(),
        r#"{"type":"xmlOnly","reason":{"type":"mp3OrAac","codec":"mp3"}}"#
    );
    let cut = Cut::Cut {
        frames: 13_009,
        seconds: Seconds(0.295),
    };
    assert_eq!(
        serde_json::to_string(&cut).unwrap(),
        r#"{"type":"cut","frames":13009,"seconds":0.295}"#
    );
    let settings = serde_json::to_string(&ExportSettings::default()).unwrap();
    assert_eq!(
        settings,
        r#"{"batchMode":"prepare","place":"inPlace","depth":null,"gridOnly":false,"tbpm":true,"xml":true,"leadMs":5.0}"#
    );
}
