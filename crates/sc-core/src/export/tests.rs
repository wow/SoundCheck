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
fn soundcheck_record_value_format() {
    let mut hash = [0xff_u8; 32];
    hash[..8].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
    let record = SoundcheckRecord {
        app: "0.1.0",
        mode: BatchMode::Prepare,
        gain: Some(RecordGain {
            stat: LoudnessMode::Dj,
            target: Lufs(-11.0),
            gain_db: -2.0,
        }),
        trim_frames: 13_009,
        sample_rate: 44_100,
        bpm: Some(Bpm(120.0)),
        bar1: Some(SampleIndex(221)),
        source_blake3: Some(hash),
    };
    assert_eq!(
        record.to_value(),
        "v=1;app=0.1.0;mode=prepare;stat=S-P95;target=-11.00;gain=-2.00;trim=13009;rate=44100;\
         bpm=120.00;bar1=221;src=0011223344556677"
    );
    let at_target = SoundcheckRecord {
        mode: BatchMode::Library,
        gain: Some(RecordGain {
            stat: LoudnessMode::Streaming,
            target: Lufs(-14.0),
            gain_db: -0.0,
        }),
        trim_frames: 0,
        sample_rate: 48_000,
        bpm: None,
        bar1: None,
        source_blake3: None,
        ..record.clone()
    };
    assert_eq!(
        at_target.to_value(),
        "v=1;app=0.1.0;mode=library;stat=I;target=-14.00;gain=+0.00;trim=0;rate=48000"
    );
    // Grid only aligned no level: it never reads as "at target".
    let grid_only = SoundcheckRecord {
        gain: None,
        trim_frames: 0,
        source_blake3: None,
        ..record
    };
    assert_eq!(
        grid_only.to_value(),
        "v=1;app=0.1.0;mode=prepare;gain=none;trim=0;rate=44100;bpm=120.00;bar1=221"
    );
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
