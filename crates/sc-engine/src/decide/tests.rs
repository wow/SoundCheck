//! Unit tests of the private parts of `crates/sc-engine/src/decide.rs`.
use super::*;
use sc_core::analysis::{Alternatives, Grid, LoudnessReport, Meter, TagHints, Timeline};
use sc_core::{AudioSpec, Bpm, DbFs, Lufs, SampleIndex, Seconds};

fn record(s_p95: Option<f64>, integrated: Option<f64>, true_peak: f64) -> AnalysisRecord {
    AnalysisRecord {
        schema: sc_core::analysis::RECORD_SCHEMA,
        version: "test".into(),
        path: "/x.flac".into(),
        size: 1,
        mtime_ns: 1,
        spec: AudioSpec::CD,
        frames: 44_100 * 200,
        duration: Seconds(200.0),
        delay: 0,
        padding: 0,
        loudness: LoudnessReport {
            integrated: integrated.map(Lufs),
            momentary_max: None,
            short_term_max: None,
            short_term_p95: s_p95.map(Lufs),
            short_term_top30: None,
            lra: None,
            true_peak: DbTp(true_peak),
            sample_peak: DbFs(true_peak),
            plr: None,
            dual_mono: false,
            timeline: Timeline {
                hop_ms: 100,
                short_term: Vec::new(),
            },
        },
        grid: None,
        grid_skipped: Some("not requested".into()),
        tags: TagHints::default(),
        evidence: None,
    }
}

fn with_grid(
    mut r: AnalysisRecord,
    bpm: f64,
    confidence: Confidence,
    verdict: Verdict,
) -> AnalysisRecord {
    r.grid = Some(Grid {
        anchor: SampleIndex(0),
        bpm: Bpm(bpm),
        meter: Meter::four_four(),
        meter_runner_up: None,
        first_downbeat_index: 0,
        phrase_len_bars: 8,
        segments: Vec::new(),
        residual_p95_ms: 5.0,
        residual_max_ms: 10.0,
        local_bpm_range: 0.0,
        drift_ppm: 0.0,
        verdict,
        confidence,
        reasons: Vec::new(),
        alternatives: Alternatives::default(),
    });
    r.grid_skipped = None;
    r
}

fn gain(plan: &Plan) -> GainPlan {
    plan.gain.expect("a gain plan")
}

/// The plan of a row whose grid the user has not confirmed.
fn unconfirmed(record: &AnalysisRecord, codec: Codec, settings: &DecideSettings) -> Plan {
    decide(record, codec, settings, false)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn loud_tracks_turn_down_whatever_their_peak() {
    let p = unconfirmed(
        &record(Some(-7.8), Some(-9.0), 1.2),
        Codec::Flac,
        &DecideSettings::dj(),
    );
    let GainPlan::Gain {
        gain_db,
        short_by_lu,
        true_peak_after,
    } = gain(&p)
    else {
        panic!("{p:?}")
    };
    assert!(close(gain_db, -3.2), "{gain_db}");
    assert!(close(short_by_lu, 0.0));
    // Still above the ceiling after turning down: shown, not limited (no limiter in v0.1).
    assert!(close(true_peak_after.0, -2.0));
    assert_eq!(p.status, JobStage::Analysed);
    assert_eq!(p.measured, Some(Lufs(-7.8)));
}

#[test]
fn a_boost_within_headroom_reaches_the_target() {
    let p = unconfirmed(
        &record(Some(-14.6), None, -5.0),
        Codec::Aiff,
        &DecideSettings::dj(),
    );
    let GainPlan::Gain {
        gain_db,
        short_by_lu,
        ..
    } = gain(&p)
    else {
        panic!()
    };
    assert!(close(gain_db, 3.6), "{gain_db}");
    assert!(close(short_by_lu, 0.0));
}

#[test]
fn the_ceiling_caps_a_boost_and_the_rest_is_short() {
    // Wants +3.6 dB, has 3.2 dB of headroom under -0.5 dBTP.
    let p = unconfirmed(
        &record(Some(-14.6), None, -3.7),
        Codec::Aiff,
        &DecideSettings::dj(),
    );
    let GainPlan::Gain {
        gain_db,
        short_by_lu,
        true_peak_after,
    } = gain(&p)
    else {
        panic!()
    };
    assert!(close(gain_db, 3.2), "{gain_db}");
    assert!(close(short_by_lu, 0.4), "{short_by_lu}");
    assert!(close(true_peak_after.0, -0.5));
}

#[test]
fn a_peak_already_over_the_ceiling_gets_no_boost() {
    let p = unconfirmed(
        &record(Some(-12.0), None, 0.3),
        Codec::Wav,
        &DecideSettings::dj(),
    );
    let GainPlan::Gain {
        gain_db,
        short_by_lu,
        ..
    } = gain(&p)
    else {
        panic!()
    };
    assert!(close(gain_db, 0.0));
    assert!(close(short_by_lu, 1.0));
}

#[test]
fn within_five_hundredths_is_at_target() {
    let p = unconfirmed(
        &record(Some(-11.04), None, -3.0),
        Codec::Flac,
        &DecideSettings::dj(),
    );
    assert_eq!(gain(&p), GainPlan::AtTarget);
}

#[test]
fn streaming_aligns_integrated_loudness() {
    let p = unconfirmed(
        &record(Some(-7.0), Some(-10.0), -1.5),
        Codec::Flac,
        &DecideSettings::streaming(),
    );
    let GainPlan::Gain { gain_db, .. } = gain(&p) else {
        panic!()
    };
    assert!(close(gain_db, -4.0), "{gain_db}");
    assert_eq!(p.measured, Some(Lufs(-10.0)));
}

#[test]
fn mp3_moves_in_whole_global_gain_steps() {
    // Down 2.2 dB: one step (1.505), residual -0.695.
    let p = unconfirmed(
        &record(Some(-8.8), None, -1.0),
        Codec::Mp3,
        &DecideSettings::dj(),
    );
    let GainPlan::GlobalGain {
        steps,
        gain_db,
        residual_lu,
        ..
    } = gain(&p)
    else {
        panic!("{p:?}")
    };
    assert_eq!(steps, -1);
    assert!(close(gain_db, -GLOBAL_GAIN_STEP_DB));
    assert!(
        close(residual_lu, -2.2 + GLOBAL_GAIN_STEP_DB),
        "{residual_lu}"
    );
    // Up 2.2 dB: rounds to one step up, which fits under the ceiling.
    let p = unconfirmed(
        &record(Some(-13.2), None, -3.0),
        Codec::Mp3,
        &DecideSettings::dj(),
    );
    let GainPlan::GlobalGain {
        steps, residual_lu, ..
    } = gain(&p)
    else {
        panic!()
    };
    assert_eq!(steps, 1);
    assert!(
        close(residual_lu, 2.2 - GLOBAL_GAIN_STEP_DB),
        "{residual_lu}"
    );
}

#[test]
fn mp3_boost_steps_stop_under_the_ceiling() {
    // Wants +4.5 dB (three steps) with 2.0 dB of headroom: one step fits.
    let p = unconfirmed(
        &record(Some(-15.5), None, -2.5),
        Codec::Mp3,
        &DecideSettings::dj(),
    );
    let GainPlan::GlobalGain {
        steps,
        true_peak_after,
        ..
    } = gain(&p)
    else {
        panic!()
    };
    assert_eq!(steps, 1);
    assert!(true_peak_after.0 <= -0.5);
}

#[test]
fn analyse_only_codecs_and_silence_are_skipped() {
    let p = unconfirmed(
        &record(Some(-10.2), None, -0.2),
        Codec::Alac,
        &DecideSettings::dj(),
    );
    assert_eq!(p.skip, Some(SkipReason::AnalyseOnly { codec: Codec::Alac }));
    assert_eq!(p.gain, None);
    assert_eq!(p.status, JobStage::Skipped);
    assert_eq!(p.measured, Some(Lufs(-10.2)), "the numbers stay visible");
    let p = unconfirmed(
        &record(None, None, -80.0),
        Codec::Flac,
        &DecideSettings::dj(),
    );
    assert_eq!(p.skip, Some(SkipReason::Silent));
    assert_eq!(p.status, JobStage::Skipped);
}

#[test]
fn a_confident_static_grid_needs_no_review() {
    let r = with_grid(
        record(Some(-9.0), None, -1.0),
        128.0,
        Confidence::Green,
        Verdict::Static,
    );
    let p = unconfirmed(&r, Codec::Flac, &DecideSettings::dj());
    assert!(p.review.is_empty(), "{:?}", p.review);
    assert_eq!(p.status, JobStage::Analysed);
}

#[test]
fn each_review_reason_is_reachable() {
    let base = record(Some(-9.0), None, -1.0);
    let dj = DecideSettings::dj();
    let r = with_grid(base.clone(), 128.0, Confidence::Amber, Verdict::StaticWarn);
    assert_eq!(
        unconfirmed(&r, Codec::Flac, &dj).review,
        vec![ReviewReason::Confidence]
    );
    let r = with_grid(base.clone(), 128.0, Confidence::Green, Verdict::StaticWarn);
    let p = unconfirmed(&r, Codec::Flac, &dj);
    assert!(
        p.review.is_empty(),
        "elevated residuals alone: {:?}",
        p.review
    );
    assert_eq!(p.status, JobStage::Analysed);
    let r = with_grid(base.clone(), 128.0, Confidence::Green, Verdict::Drifts);
    assert_eq!(
        unconfirmed(&r, Codec::Flac, &dj).review,
        vec![ReviewReason::Drifts]
    );
    let r = with_grid(base.clone(), 187.0, Confidence::Green, Verdict::Static);
    assert_eq!(
        unconfirmed(&r, Codec::Flac, &dj).review,
        vec![ReviewReason::OutsideBpmRange]
    );
    let mut r = with_grid(base.clone(), 128.0, Confidence::Green, Verdict::Static);
    r.tags.bpm = Some(Bpm(132.0));
    assert_eq!(
        unconfirmed(&r, Codec::Flac, &dj).review,
        vec![ReviewReason::TagBpmDisagrees { tag: Bpm(132.0) }]
    );
    r.tags.bpm = Some(Bpm(128.0 * 1.019));
    assert!(
        unconfirmed(&r, Codec::Flac, &dj).review.is_empty(),
        "within 2 %"
    );
    let mut r = base;
    r.grid_skipped = Some(NO_BEATS.into());
    let p = unconfirmed(&r, Codec::Flac, &dj);
    assert_eq!(p.review, vec![ReviewReason::NoGrid]);
    assert_eq!(p.status, JobStage::NeedsReview);
}

#[test]
fn a_confirmed_grid_leaves_review_whatever_its_flags() {
    let base = record(Some(-9.0), None, -1.0);
    let dj = DecideSettings::dj();
    let mut rows = vec![
        with_grid(base.clone(), 128.0, Confidence::Red, Verdict::Drifts),
        with_grid(base.clone(), 187.0, Confidence::Amber, Verdict::StaticWarn),
    ];
    rows[1].tags.bpm = Some(Bpm(93.5));
    let mut no_beats = base;
    no_beats.grid_skipped = Some(NO_BEATS.into());
    rows.push(no_beats);
    for r in &rows {
        assert_eq!(
            unconfirmed(r, Codec::Flac, &dj).status,
            JobStage::NeedsReview
        );
        let p = decide(r, Codec::Flac, &dj, true);
        assert!(p.review.is_empty(), "{:?}", p.review);
        assert_eq!(p.status, JobStage::Analysed);
        assert_eq!(
            p.gain,
            unconfirmed(r, Codec::Flac, &dj).gain,
            "loudness unchanged"
        );
    }
    // A confirmed grid does not rescue a file that is skipped.
    let p = decide(&rows[0], Codec::Alac, &dj, true);
    assert_eq!(p.status, JobStage::Skipped);
}

#[test]
fn a_short_file_without_a_grid_needs_no_review() {
    let mut r = record(Some(-9.0), None, -1.0);
    r.grid_skipped = Some("shorter than 10 s".into());
    assert!(
        unconfirmed(&r, Codec::Flac, &DecideSettings::dj())
            .review
            .is_empty()
    );
}
