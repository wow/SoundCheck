//! The plan from the snapped cut: bar 1 lands between the lead and 1 ms plus a sample after it
//! for every snap the renderer may make, every value that depends on the cut follows it, and a
//! render asked for the snapped cut gives exactly the planned output (snapping it again would
//! not).
use std::sync::atomic::AtomicBool;

use sc_core::RenderRequest;
use sc_io::render::head_snap_frames;

use super::*;
use crate::{ApplyOptions, ApplyRequest, CancelToken, Place as ApplyPlace, apply_file};

/// The plan of `record` in Prepare with a lead of `lead_ms`, and its input's parts.
fn prepared(record: &AnalysisRecord, lead_ms: f64) -> (Plan, ExportSettings, ExportPlan) {
    let decide_settings = DecideSettings::dj();
    let plan = decide(record, Codec::Wav, &decide_settings, false);
    let settings = ExportSettings {
        lead_ms,
        ..prepare()
    };
    let source = wav();
    let input = ExportInput {
        record,
        plan: &plan,
        decide: &decide_settings,
        source: &source,
    };
    let export = written(plan_export(&input, &settings));
    (plan, settings, export)
}

/// `plan_snapped_cut` for `record` planned as [`prepared`] gives.
fn snapped(
    record: &AnalysisRecord,
    plan: &Plan,
    settings: &ExportSettings,
    export: &ExportPlan,
    snapped_frames: u64,
) -> sc_core::Result<ExportPlan> {
    let decide_settings = DecideSettings::dj();
    let source = wav();
    let input = ExportInput {
        record,
        plan,
        decide: &decide_settings,
        source: &source,
    };
    plan_snapped_cut(&input, settings, export, snapped_frames)
}

/// The `bar1=` of a plan's SOUNDCHECK record.
fn bar1_of(plan: &ExportPlan) -> u64 {
    let record = tag(plan, "SOUNDCHECK").expect("record");
    let at = record.find(";bar1=").expect("bar1") + 6;
    record[at..]
        .split(';')
        .next()
        .and_then(|v| v.parse().ok())
        .expect("a number")
}

#[test]
fn bar1_lands_within_1ms_and_a_sample_after_the_lead_for_every_snap() {
    let mut checked = 0;
    for (rate, bpm, anchor) in [
        (44_100_u32, 120.0, 101_430_u64),
        (44_100, 128.0, 83_687),
        (48_000, 174.5, 10_001),
        (48_000, 122.37, 17_001),
    ] {
        for lead_ms in [0.0, 5.0, 12.3] {
            let mut r = record(rate, anchor);
            r.grid.as_mut().expect("grid").bpm = Bpm(bpm);
            let (plan, settings, export) = prepared(&r, lead_ms);
            let requested = export.trim_frames;
            assert!(requested > 0, "{rate} {bpm} {anchor}: cut");
            let bars = Bars::of(r.grid.as_ref().expect("grid"), rate).expect("bars");
            let line = bars.first_at_or_after(0.0);
            let lead = lead_ms * f64::from(rate) / 1000.0;
            let window = head_snap_frames(rate);
            let one_ms = f64::from(rate) / 1000.0;
            for s in requested - window..=requested {
                let p = snapped(&r, &plan, &settings, &export, s).expect("a snap");
                // u64 -> f64 is exact at these sizes.
                #[allow(clippy::cast_precision_loss)]
                let at = line - s as f64;
                assert!(
                    at >= lead && at < lead + one_ms + 1.0,
                    "{rate} Hz {bpm} BPM lead {lead_ms} ms snap {s}: bar 1 at {at}"
                );
                assert_eq!(p.trim_frames, s);
                assert_eq!(p.trim_snapped_from_frames, Some(requested));
                assert_eq!(p.expect_frames, r.frames - s);
                assert_eq!(
                    p.cut,
                    Cut::Cut {
                        frames: s,
                        seconds: SampleIndex(s).to_seconds(rate),
                    }
                );
                let rec = tag(&p, "SOUNDCHECK").expect("record");
                assert!(rec.contains(&format!(";trim={s};")), "{rec}");
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let rounded = at.round() as u64;
                assert_eq!(bar1_of(&p), rounded);
                // Nothing else moves.
                assert_eq!(
                    (p.gain_db, p.bits, p.bext),
                    (export.gain_db, export.bits, export.bext)
                );
                assert_eq!(p.tags.len(), export.tags.len());
                checked += 1;
            }
        }
    }
    // 1 ms is 44 frames at 44.1 kHz and 48 at 48 kHz: 45 or 49 snaps per case.
    assert_eq!(checked, 6 * 45 + 6 * 49);
}

#[test]
fn a_snap_to_the_start_cuts_nothing_and_bad_snaps_are_refused() {
    // Bar 1 at 251 samples: the cut asked is floor(251 - 220.5) = 30 frames, within 1 ms of
    // the start, so the snap may clamp at the first frame.
    let r = record(44_100, 251);
    let (plan, settings, export) = prepared(&r, 5.0);
    assert_eq!(export.trim_frames, 30);
    let p = snapped(&r, &plan, &settings, &export, 0).expect("a snap to the start");
    assert!(matches!(p.cut, Cut::OnBar { .. }), "{:?}", p.cut);
    assert_eq!((p.trim_frames, p.expect_frames), (0, r.frames));
    assert_eq!(p.trim_snapped_from_frames, Some(30));
    assert_eq!(bar1_of(&p), 251);
    let invalid =
        |e: sc_core::Result<ExportPlan>| matches!(e, Err(sc_core::Error::InvalidArgument(_)));
    // Later than asked, or more than 1 ms (44 frames) earlier.
    let r = record(44_100, 101_430);
    let (plan, settings, export) = prepared(&r, 5.0);
    assert_eq!(export.trim_frames, 13_009);
    assert!(invalid(snapped(&r, &plan, &settings, &export, 13_010)));
    assert!(invalid(snapped(&r, &plan, &settings, &export, 13_009 - 45)));
    // Snapped twice.
    let once = snapped(&r, &plan, &settings, &export, 12_965).expect("a snap");
    assert!(invalid(snapped(&r, &plan, &settings, &once, 12_921)));
    // A plan made under other settings (another lead) is not this file's plan.
    let (_, other_settings, _) = prepared(&r, 10.0);
    assert!(invalid(snapped(
        &r,
        &plan,
        &other_settings,
        &export,
        12_965
    )));
    // A plan without a cut snaps to 0 only, and stays as it is.
    let library = ExportSettings::new(BatchMode::Library);
    let uncut = written(plan_with(&r, &wav(), &library));
    assert_eq!(
        snapped(&r, &plan, &library, &uncut, 0).expect("no cut"),
        uncut
    );
    assert!(invalid(snapped(&r, &plan, &library, &uncut, 3)));
}

/// 24-bit stereo samples whose left channel holds `16 * frame index` and whose right is
/// silent: the level rises towards the end, so every snap moves a full 1 ms back, and each
/// output frame names the source frame it came from.
fn indexed_wav(path: &std::path::Path, frames: u32) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create");
    for i in 0..frames {
        w.write_sample(i32::try_from(i).expect("small") * 16)
            .expect("write");
        w.write_sample(0_i32).expect("write");
    }
    w.finalize().expect("finalize");
}

/// The left channel of a 24-bit stereo WAV.
fn left_channel(path: &std::path::Path) -> Vec<i32> {
    let mut r = hound::WavReader::open(path).expect("open");
    let all: Vec<i32> = r.samples::<i32>().map(|s| s.expect("sample")).collect();
    all.chunks(2).map(|f| f[0]).collect()
}

#[test]
fn rendering_the_snapped_plan_puts_bar1_exactly_where_it_says() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("track.wav");
    let frames = 60_000_u32;
    indexed_wav(&path, frames);
    // The grid cut at 13,009 frames above (bar line 13,230), on this file; no gain.
    let mut r = record(44_100, 101_430);
    r.frames = u64::from(frames);
    r.loudness.short_term_p95 = Some(Lufs(-11.0));
    let decide_settings = DecideSettings::dj();
    let plan = decide(&r, Codec::Wav, &decide_settings, false);
    let source = wav();
    let input = ExportInput {
        record: &r,
        plan: &plan,
        decide: &decide_settings,
        source: &source,
    };
    let outcome = crate::plan_export_snapped(&path, &input, &prepare()).expect("planned");
    let export = written(outcome);
    // The rising level moves the snap the full 44 frames back.
    assert_eq!(export.trim_snapped_from_frames, Some(13_009));
    assert_eq!(export.trim_frames, 12_965);
    assert!(export.gain_db.abs() < 1e-12, "{}", export.gain_db);
    assert_eq!(bar1_of(&export), 13_230 - 12_965);
    // Through the write transaction, as an export does, asking for exactly the snapped cut.
    let out_dir = dir.path().join("out");
    let request = ApplyRequest {
        gain_db: export.gain_db,
        trim_frames: export.trim_frames,
        trim_snapped_from_frames: export.trim_snapped_from_frames,
        ..ApplyRequest::default()
    };
    let opts = ApplyOptions {
        place: ApplyPlace::Folder(out_dir.clone()),
        ..ApplyOptions::in_place(dir.path().join("backups"))
    };
    let report = apply_file(&path, &request, &opts, &CancelToken::new()).expect("written");
    assert_eq!(
        (
            report.render.trim_frames,
            report.render.trim_requested_frames
        ),
        (12_965, 13_009)
    );
    assert_eq!(report.render.frames_out, export.expect_frames);
    let out = left_channel(&out_dir.join("track.wav"));
    assert_eq!(out.len() as u64, export.expect_frames);
    // Past the 2 ms fade every frame is its source frame: output frame k is source k + 12,965,
    // so the bar line at source 13,230 is output frame 265, the planned bar 1.
    let fade = sc_io::render::head_fade_frames(44_100);
    for (k, v) in out.iter().enumerate().skip(fade) {
        let source = usize::try_from(*v / 16).expect("a frame index");
        assert_eq!(source - k, 12_965, "output frame {k}");
    }
    let sidecar = std::fs::read_to_string(sc_io::txn::sidecar_path(&out_dir.join("track.wav")))
        .expect("sidecar");
    assert!(
        sidecar.contains("\"trim_snapped_from_frames\": 13009"),
        "{sidecar}"
    );
    assert!(sidecar.contains("\"trim_frames\": 12965"), "{sidecar}");
    // Snapping the snapped cut again would move it another 1 ms back: not the plan.
    let again = sc_io::apply_iff(
        &path,
        &dir.path().join("again.wav"),
        &RenderRequest {
            trim_frames: export.trim_frames,
            ..RenderRequest::default()
        },
        &AtomicBool::new(false),
    )
    .expect("rendered");
    assert_eq!(again.trim_frames, 12_921);
}

#[test]
fn the_serato_in_place_skip_follows_the_cut_made() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("track.wav");
    indexed_wav(&path, 60_000);
    let decide_settings = DecideSettings::dj();
    let outcomes = |anchor: u64, serato: SeratoPresence| {
        let mut r = record(44_100, anchor);
        r.frames = 60_000;
        let plan = decide(&r, Codec::Wav, &decide_settings, false);
        let source = ExportSource { serato, ..wav() };
        let input = ExportInput {
            record: &r,
            plan: &plan,
            decide: &decide_settings,
            source: &source,
        };
        let snapped = crate::plan_export_snapped(&path, &input, &prepare()).expect("planned");
        (plan_export(&input, &prepare()), snapped)
    };
    // Bar 1 at 251 samples: 30 frames are asked, and the snap moves the cut to the first
    // frame (the quietest), so nothing is cut and no Serato cue point moves.
    let (asked, made) = outcomes(251, SeratoPresence::Present);
    assert!(
        matches!(
            asked,
            ExportOutcome::Skip {
                reason: ExportSkip::SeratoInPlaceCut { .. }
            }
        ),
        "{asked:?}"
    );
    let plan = written(made);
    assert_eq!(
        (plan.trim_frames, plan.trim_snapped_from_frames),
        (0, Some(30))
    );
    assert!(matches!(plan.cut, Cut::OnBar { .. }), "{:?}", plan.cut);
    assert_eq!(plan.notices, Vec::<ExportNotice>::new());
    // A real cut is still refused, by the cut made: 12,965 frames.
    for (serato, expect) in [
        (
            SeratoPresence::Present,
            ExportSkip::SeratoInPlaceCut {
                cut_s: SampleIndex(12_965).to_seconds(44_100),
            },
        ),
        (
            SeratoPresence::Unknown,
            ExportSkip::SeratoUnknownInPlaceCut {
                cut_s: SampleIndex(12_965).to_seconds(44_100),
            },
        ),
    ] {
        let (_, made) = outcomes(101_430, serato);
        assert_eq!(made, ExportOutcome::Skip { reason: expect });
    }
}
