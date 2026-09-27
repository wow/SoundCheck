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
    };
    assert!(json_len(&header) <= 4096, "{} bytes", json_len(&header));
}

#[test]
fn meter_frame_stays_within_budget() {
    let frame = MeterFrame {
        file_id: u32::MAX,
        momentary: Lufs(-123.456_789),
        short_term: Lufs(-123.456_789),
        true_peak: DbTp(-123.456_789),
    };
    assert!(
        json_len(&frame) <= MAX_EVENT_BYTES,
        "{} bytes",
        json_len(&frame)
    );
}

#[test]
fn errors_map_to_their_class() {
    let ipc: IpcError = crate::Error::Cancelled.into();
    assert_eq!(ipc.kind, IpcErrorKind::Cancelled);
    assert_eq!(ipc.message, "cancelled");
    assert_eq!(
        serde_json::to_string(&JobStage::NeedsReview).unwrap(),
        "\"needsReview\""
    );
}
