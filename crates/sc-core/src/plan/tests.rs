//! Unit tests of the private parts of `crates/sc-core/src/plan.rs`.
use super::*;

#[test]
fn global_gain_step_is_a_quarter_power_of_two() {
    let step = 20.0 * 2f64.powf(0.25).log10();
    assert!((step - GLOBAL_GAIN_STEP_DB).abs() < 1e-12, "{step}");
}

#[test]
fn settings_outside_their_limits_are_rejected() {
    assert!(DecideSettings::dj().validate().is_ok());
    assert!(DecideSettings::streaming().validate().is_ok());
    let above_full_scale = DecideSettings {
        ceiling: DbTp(0.5),
        ..DecideSettings::dj()
    };
    assert!(above_full_scale.validate().is_err());
    let too_loud = DecideSettings {
        target: Lufs(-2.0),
        ..DecideSettings::dj()
    };
    assert!(too_loud.validate().is_err());
    let nan = DecideSettings {
        target: Lufs(f64::NAN),
        ..DecideSettings::dj()
    };
    assert!(nan.validate().is_err());
    assert!(check_bpm_range((Bpm(180.0), Bpm(70.0))).is_err());
    assert!(check_bpm_range((Bpm(20.0), Bpm(70.0))).is_err());
    assert!(check_bpm_range((Bpm(80.0), Bpm(160.0))).is_ok());
}

#[test]
fn codecs_from_names() {
    assert_eq!(Codec::from_path(Path::new("a/B.FLAC")), Codec::Flac);
    assert_eq!(Codec::from_path(Path::new("x.aif")), Codec::Aiff);
    assert_eq!(Codec::from_path(Path::new("x.m4a")), Codec::Aac);
    assert_eq!(Codec::from_path(Path::new("x")), Codec::Other);
    assert!(Codec::Flac.is_writable() && Codec::Aiff.is_writable() && Codec::Wav.is_writable());
    assert!(!Codec::Mp3.is_writable() && !Codec::Aac.is_writable());
    assert!(Codec::Mp3.has_gain_plan() && Codec::Wav.has_gain_plan());
    assert!(!Codec::Alac.is_writable() && !Codec::Alac.has_gain_plan());
}

#[test]
fn plans_serialise_as_tagged_unions() {
    let gain = GainPlan::Gain {
        gain_db: -3.2,
        short_by_lu: 0.0,
        true_peak_after: DbTp(-3.5),
    };
    assert_eq!(
        serde_json::to_string(&gain).unwrap(),
        r#"{"type":"gain","gainDb":-3.2,"shortByLu":0.0,"truePeakAfter":-3.5}"#
    );
    assert_eq!(
        serde_json::to_string(&SkipReason::AnalyseOnly { codec: Codec::Alac }).unwrap(),
        r#"{"type":"analyseOnly","codec":"alac"}"#
    );
}
