//! Unit tests of the private parts of `crates/sc-core/src/ipc/mod.rs`.
use super::*;

fn json_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).expect("serialisable").len()
}

#[test]
fn progress_event_stays_within_budget() {
    let ev = JobEvent::Progress {
        job_id: u32::MAX,
        file_id: u32::MAX,
        fraction: 0.333_333_34,
    };
    assert!(json_len(&ev) <= MAX_EVENT_BYTES, "{} bytes", json_len(&ev));
    let ev = JobEvent::Batch {
        job_id: u32::MAX,
        done: u32::MAX,
        total: u32::MAX,
        eta_ms: Some(u32::MAX),
        realtime_x: Some(123.456_79),
    };
    assert!(json_len(&ev) <= MAX_EVENT_BYTES, "{} bytes", json_len(&ev));
}

#[test]
fn a_worst_case_analysed_row_stays_within_budget() {
    use crate::analysis::Meter;
    use crate::plan::{GainPlan, ReviewReason};
    let grid = RowGrid {
        bpm: Bpm(123.456_789),
        meter: Meter::ten_eight().to_string(),
        four_four: false,
        meter_runner_up: Some(Meter::nine_eight_long_first().to_string()),
        confidence: Confidence::Red,
        reasons: vec![
            Reason::Residuals,
            Reason::Coverage,
            Reason::Recall,
            Reason::OctaveMargin,
            Reason::DownbeatMargin,
            Reason::MeterMargin,
            Reason::TagDisagrees,
            Reason::OutsideRange,
            Reason::Short,
            Reason::Drifts,
            Reason::NoKick,
            Reason::Manual,
        ],
        verdict: Verdict::Drifts,
        octave_up: Some(Bpm(246.913_578)),
        octave_down: Some(Bpm(61.728_394)),
        bar1: Seconds(1.234_567_89),
        residual_p95_ms: 12.345_678,
        residual_max_ms: 45.678_9,
        drift_ppm: -1_083.697_9,
    };
    let row = RowAnalysis {
        duration: Seconds(612.345_678_9),
        sample_rate: 192_000,
        channels: 2,
        integrated: Some(Lufs(-12.345_678_9)),
        short_term_p95: Some(Lufs(-10.123_456_7)),
        lra: Some(Lu(12.345_678_9)),
        true_peak: DbTp(-0.123_456_789),
        grid: Some(grid),
        grid_skipped: None,
        tag_bpm: Some(Bpm(123.45)),
        cached: false,
        edited: true,
        confirmed: true,
    };
    let plan = Plan {
        measured: Some(Lufs(-10.123_456_7)),
        gain: Some(GainPlan::GlobalGain {
            steps: -3,
            gain_db: -4.515_449_934_959_718,
            residual_lu: 0.392_006_765_040_282,
            true_peak_after: DbTp(-4.638_906_723_959_718),
        }),
        skip: None,
        review: vec![
            ReviewReason::Confidence,
            ReviewReason::Drifts,
            ReviewReason::OutsideBpmRange,
            ReviewReason::TagBpmDisagrees { tag: Bpm(123.45) },
        ],
        status: JobStage::NeedsReview,
    };
    let ev = JobEvent::Analysed {
        job_id: u32::MAX,
        file_id: u32::MAX,
        row: Box::new(row),
        plan,
        revision: u32::MAX,
    };
    assert!(json_len(&ev) <= MAX_ROW_BYTES, "{} bytes", json_len(&ev));
}

#[test]
fn a_worst_case_refit_header_stays_within_4_kb() {
    use crate::analysis::{Alternatives, Grid, Meter};
    let grid = Grid {
        anchor: crate::SampleIndex(u64::MAX),
        bpm: Bpm(123.456_789_012),
        meter: Meter::ten_eight(),
        meter_runner_up: Some(Meter::nine_eight_long_first()),
        first_downbeat_index: u32::MAX,
        phrase_len_bars: 8,
        segments: Vec::new(),
        residual_p95_ms: 12.345_678,
        residual_max_ms: 45.678_9,
        local_bpm_range: 0.123_456_7,
        drift_ppm: -1_083.697_9,
        verdict: Verdict::Drifts,
        confidence: Confidence::Red,
        reasons: vec![
            Reason::Residuals,
            Reason::Coverage,
            Reason::Recall,
            Reason::OctaveMargin,
            Reason::DownbeatMargin,
            Reason::MeterMargin,
            Reason::TagDisagrees,
            Reason::OutsideRange,
            Reason::Short,
            Reason::Drifts,
            Reason::NoKick,
            Reason::Manual,
        ],
        alternatives: Alternatives {
            octave_up: Some(Bpm(246.913_578_024)),
            octave_down: Some(Bpm(61.728_394_506)),
            downbeat_shift_beats: vec![-4, -3, -2, -1, 1, 2, 3, 4, 5],
        },
    };
    let header = GridFitHeader {
        grid: Some(grid),
        first_line: i64::MIN,
        lines: u32::MAX,
        worst_line: Some(i64::MAX),
        matched: u32::MAX,
        attacks: u32::MAX,
        fit_choice: Some(FitChoice {
            whole_share: 0.123_456_79,
            start_share: 0.987_654_3,
            window_end_s: 123_456.789_012_345,
        }),
    };
    assert!(json_len(&header) <= 4096, "{} bytes", json_len(&header));
}

/// The largest meter frame the player sends: every level present, rounded to 0.001 dB as the
/// engine rounds them, at the largest position a JavaScript number holds exactly.
fn worst_meter_frame() -> MeterFrame {
    MeterFrame {
        position: crate::SampleIndex(9_007_199_254_740_991),
        in_peak: Some(DbTp(-123.457)),
        in_momentary: Some(Lufs(-123.457)),
        out_peak: Some(DbTp(-123.457)),
        out_momentary: Some(Lufs(-123.457)),
        folded: false,
    }
}

#[test]
fn meter_frame_stays_within_budget() {
    let frame = worst_meter_frame();
    assert!(json_len(&frame) <= 176, "{} bytes", json_len(&frame));
}

#[test]
fn a_player_event_with_its_meter_stays_within_budget() {
    // Ten hours at 192 kHz, every level present with three decimals.
    let ten_hours = crate::SampleIndex(10 * 3_600 * 192_000);
    let event = TrackEvent::Player {
        playing: true,
        position: ten_hours,
        underruns: u64::from(u32::MAX),
        meter: Some(MeterFrame {
            position: ten_hours,
            ..worst_meter_frame()
        }),
        listen: Listen {
            version: Version::Processed,
            matched: true,
        },
    };
    assert!(
        json_len(&event) <= MAX_EVENT_BYTES,
        "{} bytes",
        json_len(&event)
    );
}

#[test]
fn listen_has_the_shape_the_view_sends() {
    let listen: Listen =
        serde_json::from_str(r#"{"version":"original","matched":true}"#).expect("parses");
    assert_eq!(
        listen,
        Listen {
            version: Version::Original,
            matched: true
        }
    );
    assert_eq!(
        Listen::default(),
        Listen {
            version: Version::Processed,
            matched: false
        }
    );
    let frame = MeterFrame {
        in_momentary: None,
        ..worst_meter_frame()
    };
    let json = serde_json::to_string(&frame).expect("serialisable");
    assert!(json.contains(r#""inMomentary":null"#), "{json}");
    assert!(json.contains(r#""outPeak":-123.457"#), "{json}");
}

#[test]
fn errors_map_to_their_class() {
    let ipc: IpcError = crate::Error::Cancelled.into();
    assert_eq!(ipc.kind, IpcErrorKind::Cancelled);
    assert_eq!(ipc.message, "cancelled");
    let ipc: IpcError = crate::Error::NotDjSafe {
        path: "a.wav".into(),
        reason: "sample rate 96000 Hz".into(),
    }
    .into();
    assert_eq!(ipc.kind, IpcErrorKind::NotDjSafe);
    assert_eq!(serde_json::to_string(&ipc.kind).unwrap(), "\"notDjSafe\"");
    assert_eq!(
        ipc.message,
        "a.wav cannot be written DJ-safe: sample rate 96000 Hz"
    );
    assert_eq!(
        serde_json::to_string(&JobStage::NeedsReview).unwrap(),
        "\"needsReview\""
    );
}

#[test]
fn transaction_errors_map_to_their_class_with_what_to_do() {
    let ipc: IpcError = crate::Error::InPlaceRefused {
        path: "a.wav".into(),
        reason: crate::InPlaceRefusal::HardLinked { links: 2 },
    }
    .into();
    assert_eq!(ipc.kind, IpcErrorKind::InPlaceRefused);
    assert_eq!(
        ipc.message,
        "a.wav cannot be changed in place: it has 2 hard links, which replacing it would break. \
         Write a copy to a folder"
    );
    let ipc: IpcError = crate::Error::NoSpace {
        volume: "/Volumes/USB".into(),
        needed_bytes: 100,
        free_bytes: 10,
    }
    .into();
    assert_eq!(ipc.kind, IpcErrorKind::NoSpace);
    assert_eq!(serde_json::to_string(&ipc.kind).unwrap(), "\"noSpace\"");
    let kinds = [
        (
            crate::Error::RekordboxUsbExport {
                path: "a".into(),
                volume: "/Volumes/U".into(),
            },
            "\"rekordboxUsbExport\"",
        ),
        (
            crate::Error::VerifyFailed {
                path: "a".into(),
                detail: "d".into(),
            },
            "\"verifyFailed\"",
        ),
        (
            crate::Error::FileChanged {
                path: "a".into(),
                detail: "d".into(),
                cause: crate::ChangeCause::DuringProcessing,
            },
            "\"fileChanged\"",
        ),
        (
            crate::Error::NothingToUndo { path: "a".into() },
            "\"nothingToUndo\"",
        ),
        (
            crate::Error::AlreadyExists { path: "a".into() },
            "\"alreadyExists\"",
        ),
        (
            crate::Error::ListedTwice {
                path: "./a".into(),
                first: "a".into(),
            },
            "\"listedTwice\"",
        ),
        (
            crate::Error::SameOutputName {
                path: "x/a".into(),
                other: "y/a".into(),
                output: "out/a".into(),
            },
            "\"sameOutputName\"",
        ),
    ];
    for (err, json) in kinds {
        assert_eq!(serde_json::to_string(&err.kind()).unwrap(), json);
    }
}

#[test]
fn recovery_status_is_tagged_by_state() {
    let status = RecoveryStatus::Finished {
        recovered: vec![RecoveredChange {
            txn: "6ac5b4f4-1-0".into(),
            path: "/m/a.wav".into(),
            outcome: RecoveryOutcome::RolledBack,
        }],
        pending: vec![],
    };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["state"], "finished");
    assert_eq!(json["recovered"][0]["outcome"], "rolledBack");
    let running = serde_json::to_value(RecoveryStatus::Running).unwrap();
    assert_eq!(running, serde_json::json!({ "state": "running" }));
}
