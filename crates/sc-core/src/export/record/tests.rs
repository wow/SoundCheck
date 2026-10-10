//! Unit tests of `crates/sc-core/src/export/record.rs`: the `SOUNDCHECK` value as written, and
//! read back exactly.
#![allow(clippy::float_cmp)] // values read back must be the very values written
use proptest::prelude::*;

use super::*;

fn prepared() -> SoundcheckRecord {
    let mut hash = [0xff_u8; 32];
    hash[..8].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
    SoundcheckRecord {
        app: "0.1.0".into(),
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
        source_hash: Some(SoundcheckRecord::hash_prefix(&hash)),
    }
}

const PREPARED: &str = "v=1;app=0.1.0;mode=prepare;stat=S-P95;target=-11.00;gain=-2.00;\
                        trim=13009;rate=44100;bpm=120.00;bar1=221;src=0011223344556677";

#[test]
fn soundcheck_record_value_format() {
    let record = prepared();
    assert_eq!(record.to_value(), PREPARED);
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
        source_hash: None,
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
        source_hash: None,
        ..record
    };
    assert_eq!(
        grid_only.to_value(),
        "v=1;app=0.1.0;mode=prepare;gain=none;trim=0;rate=44100;bpm=120.00;bar1=221"
    );
}

#[test]
fn soundcheck_record_round_trip() {
    let record = prepared();
    assert_eq!(SoundcheckRecord::parse(PREPARED), Ok(record.clone()));
    for value in [
        "v=1;app=0.1.0;mode=library;stat=I;target=-14.00;gain=+0.00;trim=0;rate=48000",
        "v=1;app=0.1.0;mode=prepare;gain=none;trim=0;rate=44100;bpm=120.00;bar1=221",
        PREPARED,
    ] {
        let parsed = SoundcheckRecord::parse(value).expect(value);
        assert_eq!(parsed.to_value(), value);
    }
    // Keys in another order, unknown keys and surrounding white space are accepted.
    let shuffled = "src=0011223344556677;bar1=221;bpm=120.00;rate=44100;trim=13009;\
                    gain=-2.00;target=-11.00;stat=S-P95;mode=prepare;app=0.1.0;v=1;\
                    lead=5.00;future=x=y";
    assert_eq!(
        SoundcheckRecord::parse(&format!(" {shuffled}\n")),
        Ok(record)
    );
}

#[test]
fn records_that_do_not_read_say_why() {
    let replace = |key: &str, value: Option<&str>| -> String {
        PREPARED
            .split(';')
            .filter_map(|part| {
                let (k, v) = part.split_once('=').expect("pair");
                if k == key {
                    value.map(|new| format!("{k}={new}"))
                } else {
                    Some(format!("{k}={v}"))
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    };
    let err = |v: &str| SoundcheckRecord::parse(v).expect_err(v);
    assert_eq!(
        err(&replace("v", Some("2"))),
        RecordError::UnsupportedVersion("2".into())
    );
    assert_eq!(err(&replace("v", None)), RecordError::Missing("v"));
    for key in ["app", "mode", "gain", "trim", "rate", "stat", "target"] {
        assert_eq!(err(&replace(key, None)), RecordError::Missing(key), "{key}");
    }
    assert_eq!(
        err(&format!("{PREPARED};trim=1")),
        RecordError::Duplicate("trim".into())
    );
    assert_eq!(
        err(&format!("{PREPARED};junk")),
        RecordError::NotAPair("junk".into())
    );
    for (key, bad) in [
        ("mode", "warp"),
        ("stat", "LUFS"),
        ("target", "loud"),
        ("gain", "NaN"),
        ("gain", "inf"),
        ("trim", "-1"),
        ("trim", "+5"),
        ("rate", "4.41e4"),
        ("bpm", "fast"),
        ("bar1", "1.5"),
        ("src", "00112233445566"),
        ("src", "001122334455667g"),
        ("app", ""),
    ] {
        assert_eq!(
            err(&replace(key, Some(bad))),
            RecordError::Invalid {
                key: KEYS.iter().find(|k| **k == key).expect("key"),
                value: bad.into()
            },
            "{key}={bad}"
        );
    }
    // A gain of none aligned no statistic.
    let none = replace("gain", Some("none"));
    assert_eq!(
        err(&none),
        RecordError::Invalid {
            key: "stat",
            value: "S-P95".into()
        }
    );
    let long = format!("{PREPARED};pad={}", "x".repeat(MAX_RECORD_BYTES));
    assert_eq!(err(&long), RecordError::TooLong(long.len()));
    assert_eq!(err(""), RecordError::NotAPair(String::new()));
}

#[test]
fn records_serialise_for_the_ui() {
    let json = serde_json::to_value(prepared()).expect("json");
    assert_eq!(json["trimFrames"], 13_009);
    assert_eq!(json["gain"]["gainDb"], -2.0);
    assert_eq!(json["sourceHash"][1], 0x11);
}

/// A tempo or level with two decimals, as the planner writes them.
fn hundredths(lo: i64, hi: i64) -> impl Strategy<Value = f64> {
    #[allow(clippy::cast_precision_loss)] // small integers are exact in f64
    (lo..=hi).prop_map(|c| c as f64 / 100.0)
}

fn any_record() -> impl Strategy<Value = SoundcheckRecord> {
    let gain = (
        prop_oneof![Just(LoudnessMode::Dj), Just(LoudnessMode::Streaming)],
        hundredths(-3_000, 0),
        hundredths(-6_000, 6_000),
    )
        .prop_map(|(stat, target, gain_db)| RecordGain {
            stat,
            target: Lufs(target),
            gain_db,
        });
    (
        "[0-9a-z.+-]{1,20}",
        prop_oneof![Just(BatchMode::Prepare), Just(BatchMode::Library)],
        proptest::option::of(gain),
        any::<u64>(),
        any::<u32>(),
        proptest::option::of(hundredths(1, 99_999)),
        proptest::option::of(any::<u64>()),
        proptest::option::of(any::<[u8; 8]>()),
    )
        .prop_map(
            |(app, mode, gain, trim_frames, sample_rate, bpm, bar1, source_hash)| {
                SoundcheckRecord {
                    app,
                    mode,
                    gain,
                    trim_frames,
                    sample_rate,
                    bpm: bpm.map(Bpm),
                    bar1: bar1.map(SampleIndex),
                    source_hash,
                }
            },
        )
}

proptest! {
    #[test]
    fn soundcheck_record_round_trip_generated(record in any_record()) {
        let value = record.to_value();
        let parsed = SoundcheckRecord::parse(&value).expect("reads back");
        prop_assert_eq!(parsed.to_value(), value);
        // Two-decimal values come back as the same numbers.
        prop_assert_eq!(parsed, record);
    }
}
