//! The Prepare cut: which bar line the output starts on, how many frames go, and when nothing
//! is cut. Sample counts are exact.
use super::*;

fn cut(frames: u64, rate: u32) -> Cut {
    Cut::Cut {
        frames,
        seconds: SampleIndex(frames).to_seconds(rate),
    }
}

/// Starts on the bar line `line`, with bar 1 at `bar1`.
fn on_bar(line: u64, bar1: u64, rate: u32) -> Cut {
    Cut::OnBar {
        bar_line: SampleIndex(line),
        bar_line_s: SampleIndex(line).to_seconds(rate),
        bar1: SampleIndex(bar1),
        bar1_s: SampleIndex(bar1).to_seconds(rate),
    }
}

#[test]
fn prepare_cuts_lead_before_anchor() {
    // Bar 1 at 2.300 s: the bar line one bar earlier, 0.300 s (13,230), is the first in the
    // file; floor(13,230 - 220.5) = 13,009 frames are cut, and the output's bar 1 sits at 221
    // samples (5.011 ms), never before the lead.
    let r = record(44_100, 101_430);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert_eq!(plan.cut, cut(13_009, 44_100));
    assert_eq!(plan.trim_frames, 13_009);
    assert_eq!(plan.expect_frames, r.frames - 13_009);
    assert!((plan.gain_db + 2.0).abs() < 1e-12, "{}", plan.gain_db);
    assert_eq!(plan.bits, None);
    assert_eq!(plan.notices, Vec::<ExportNotice>::new());
    let record = tag(&plan, "SOUNDCHECK").expect("record");
    assert!(
        record.contains(";rate=44100;bpm=120.00;bar1=221"),
        "{record}"
    );
    // Bar 1 itself as the first bar line.
    assert_eq!(cut_of(44_100, 13_230), cut(13_009, 44_100));
}

#[test]
fn fractional_bar_lines() {
    // 128 BPM at 44.1 kHz: a bar is 82,687.5 samples. Bar 1 at 83,687 puts the first bar line
    // at 999.5; floor(999.5 - 220.5) = 779 frames are cut and bar 1 lands at 220.5 samples,
    // written as 221.
    let mut r = record(44_100, 83_687);
    r.grid.as_mut().expect("grid").bpm = Bpm(128.0);
    let bars = Bars::of(r.grid.as_ref().expect("grid"), 44_100).expect("bars");
    assert!((bars.bar - 82_687.5).abs() < 1e-9, "{}", bars.bar);
    assert!((bars.first_at_or_after(0.0) - 999.5).abs() < 1e-9);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert_eq!(plan.cut, cut(779, 44_100));
    assert!((bars.first_at_or_after(779.0) - 779.0 - 220.5).abs() < 1e-9);
    assert!(
        tag(&plan, "SOUNDCHECK")
            .expect("record")
            .contains(";bar1=221")
    );
}

#[test]
fn bar_lines_use_the_written_bpm() {
    // A fitted 119.996 BPM is written as 120.00, so bar lines are laid at 120.00: the same cut
    // as at 120 (at 119.996 the line would sit 3 samples earlier and 13,006 would be cut).
    let mut r = record(44_100, 101_430);
    r.grid.as_mut().expect("grid").bpm = Bpm(119.996);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert_eq!(plan.cut, cut(13_009, 44_100));
    assert_eq!(tag(&plan, "BPM"), Some("120.00"));
    assert!(
        tag(&plan, "SOUNDCHECK")
            .expect("record")
            .contains(";bpm=120.00;")
    );
}

#[test]
fn prepare_trim_zero_when_bar1_at_lead() {
    // Bar 1 exactly at the lead (240 samples at 48 kHz): it already starts there.
    assert_eq!(cut_of(48_000, 240), on_bar(240, 240, 48_000));
    // Two bars later: the extrapolated bar line is at the lead.
    assert_eq!(
        cut_of(48_000, 240 + 2 * 96_000),
        on_bar(240, 240 + 2 * 96_000, 48_000)
    );
    let plan = written(plan_with(&record(48_000, 240), &wav(), &prepare()));
    assert_eq!((plan.trim_frames, plan.expect_frames), (0, 48_000 * 240));
}

#[test]
fn a_file_that_starts_on_a_bar_line_is_not_cut() {
    // Bar 1 on the first sample (a promo that starts on the downbeat).
    assert_eq!(cut_of(48_000, 0), on_bar(0, 0, 48_000));
    // One sample before the lead.
    assert_eq!(cut_of(48_000, 239), on_bar(239, 239, 48_000));
    // Bar 1 at 8.000 s at 120 BPM: four whole bars before it, a bar line on the first sample;
    // the cut names that line and bar 1 itself, which is not where the file starts.
    let at_8s = cut_of(44_100, 352_800);
    assert_eq!(at_8s, on_bar(0, 352_800, 44_100));
    let Cut::OnBar { bar1_s, .. } = at_8s else {
        panic!("{at_8s:?}");
    };
    assert!((bar1_s.0 - 8.0).abs() < 1e-12);
    let plan = written(plan_with(&record(44_100, 352_800), &wav(), &prepare()));
    assert_eq!((plan.trim_frames, plan.expect_frames), (0, 44_100 * 240));
    assert!(
        tag(&plan, "SOUNDCHECK")
            .expect("record")
            .contains(";trim=0;rate=44100;bpm=120.00;bar1=0")
    );
}

#[test]
fn prepare_not_cut_when_first_bar_line_more_than_a_beat_in() {
    // Bar 1 at 1.000 s: cutting to it would remove 0.995 s, more than a 0.5 s beat.
    let r = record(44_100, 44_100);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    let one_second = SampleIndex(44_100);
    assert_eq!(
        plan.cut,
        Cut::NotCut {
            bar1: one_second,
            bar1_s: Seconds(1.0),
            first_bar_line: one_second,
            first_bar_line_s: Seconds(1.0),
        }
    );
    assert_eq!((plan.trim_frames, plan.expect_frames), (0, r.frames));
    // Bar 1 at 9.000 s: reported with the bar line nearest the start, 1.000 s.
    assert_eq!(
        cut_of(44_100, 396_900),
        Cut::NotCut {
            bar1: SampleIndex(396_900),
            bar1_s: Seconds(9.0),
            first_bar_line: one_second,
            first_bar_line_s: Seconds(1.0),
        }
    );
    // Exactly one beat is still cut; one sample more is not.
    assert_eq!(cut_of(48_000, 240 + 24_000), cut(24_000, 48_000));
    assert!(matches!(cut_of(48_000, 240 + 24_001), Cut::NotCut { .. }));
}

#[test]
fn odd_meter_bar_length() {
    // 9/8 counted in eighths at 300 per minute, 48 kHz: a pulse is 9,600 samples, the bar
    // 86,400, and the last beat (the group of three) 28,800. 300 lies outside the DJ app's BPM
    // range, so the row needs review: the grid is confirmed here, else nothing would be cut.
    let mut r = record(48_000, 240 + 86_400 + 28_800);
    let grid = r.grid.as_mut().expect("grid");
    grid.meter = Meter::new(BeatUnit::Eighth, &[2, 2, 2, 3]);
    grid.bpm = Bpm(300.0);
    let bars = Bars::of(grid, 48_000).expect("bars");
    assert!((bars.bar - 86_400.0).abs() < 1e-9 && (bars.last_beat - 28_800.0).abs() < 1e-9);
    let plan = written(plan_decided(&r, &wav(), &prepare(), true));
    assert_eq!(plan.trim_frames, 28_800);
    assert_eq!(tag(&plan, "BPM"), Some("300.00"));
    // With the long group first, the last beat is two pulses: the same cut is too long.
    r.grid.as_mut().expect("grid").meter = Meter::new(BeatUnit::Eighth, &[3, 2, 2, 2]);
    let plan = written(plan_decided(&r, &wav(), &prepare(), true));
    assert!(matches!(plan.cut, Cut::NotCut { .. }), "{:?}", plan.cut);
}

#[test]
fn a_grid_that_needs_review_is_not_cut_unless_confirmed() {
    // The grid that is cut at 13,009 frames above, with amber confidence: the row needs review.
    let mut r = record(44_100, 101_430);
    r.grid.as_mut().expect("grid").confidence = Confidence::Amber;
    let decide_settings = DecideSettings::dj();
    let source = wav();
    let plan_of = |confirmed: bool| {
        let plan = decide(&r, source.codec, &decide_settings, confirmed);
        let input = ExportInput {
            record: &r,
            plan: &plan,
            decide: &decide_settings,
            source: &source,
        };
        (plan.status, written(plan_export(&input, &prepare())))
    };
    let (status, plan) = plan_of(false);
    assert_eq!(status, JobStage::NeedsReview);
    assert_eq!(
        plan.cut,
        Cut::NeedsReview {
            bar1: SampleIndex(101_430),
            bar1_s: SampleIndex(101_430).to_seconds(44_100),
        }
    );
    assert_eq!((plan.trim_frames, plan.expect_frames), (0, r.frames));
    // Gain and the loudness tags are still written; the grid is not: no tempo tag, and the
    // record says nothing was cut and vouches for no grid.
    assert!((plan.gain_db + 2.0).abs() < 1e-12, "{}", plan.gain_db);
    assert_eq!(tag(&plan, "BPM"), None);
    assert!(tag(&plan, "REPLAYGAIN_TRACK_GAIN").is_some());
    let rec = tag(&plan, "SOUNDCHECK").expect("record");
    assert!(
        rec.contains(";trim=0;rate=44100;bpm=none;bar1=none"),
        "{rec}"
    );
    // Confirmed by ear: the same grid is cut and written.
    let (status, plan) = plan_of(true);
    assert_eq!(status, JobStage::Analysed);
    assert_eq!(plan.cut, cut(13_009, 44_100));
    assert_eq!(tag(&plan, "BPM"), Some("120.00"));
    let rec = tag(&plan, "SOUNDCHECK").expect("record");
    assert!(rec.contains(";bpm=120.00;bar1=221"), "{rec}");
    // Any reason to review counts (here a BPM tag that disagrees), and Library and grid only
    // keep their own wording; Library does not write a grid that needs review either.
    let mut tagged = record(44_100, 101_430);
    tagged.tags.bpm = Some(Bpm(126.0));
    assert!(matches!(
        written(plan_with(&tagged, &wav(), &prepare())).cut,
        Cut::NeedsReview { .. }
    ));
    let library = ExportSettings {
        tbpm: true,
        ..ExportSettings::new(BatchMode::Library)
    };
    let plan = written(plan_with(&tagged, &wav(), &library));
    assert_eq!(plan.cut, Cut::Library);
    assert_eq!(tag(&plan, "BPM"), None);
}

#[test]
fn a_disagreeing_bpm_tag_survives_an_export_so_the_row_still_needs_review() {
    // The file's tag says 126 BPM, the grid 120: the row needs review.
    let mut r = record(44_100, 101_430);
    r.tags.bpm = Some(Bpm(126.0));
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert!(
        matches!(plan.cut, Cut::NeedsReview { .. }),
        "{:?}",
        plan.cut
    );
    assert_eq!(
        tag(&plan, "BPM"),
        None,
        "the 126 tag is not replaced by 120.00"
    );
    let rec = tag(&plan, "SOUNDCHECK").expect("record");
    assert!(rec.contains(";bpm=none;bar1=none"), "{rec}");
    // Analysed again after the export: the file's tempo tag is whatever the export left (a
    // written tempo would have replaced 126). It still needs review, so the next export does
    // not cut either.
    let mut again = r.clone();
    if let Some(bpm) = tag(&plan, "BPM") {
        again.tags.bpm = Some(Bpm(bpm.parse().expect("a number")));
    }
    let status = decide(&again, Codec::Wav, &DecideSettings::dj(), false).status;
    assert_eq!(status, JobStage::NeedsReview);
    let next = written(plan_with(&again, &wav(), &prepare()));
    assert_eq!(next.trim_frames, 0);
    assert!(
        matches!(next.cut, Cut::NeedsReview { .. }),
        "{:?}",
        next.cut
    );
}

#[test]
fn grid_only_on_a_review_row_is_skipped_not_written() {
    // Amber confidence: the row needs review. Grid only would write no tempo, no gain and a
    // record that withholds the grid: nothing worth rewriting the file for.
    let mut r = record(44_100, 101_430);
    r.grid.as_mut().expect("grid").confidence = Confidence::Amber;
    let grid_only = ExportSettings {
        grid_only: true,
        ..prepare()
    };
    assert_eq!(
        plan_with(&r, &wav(), &grid_only),
        ExportOutcome::Skip {
            reason: ExportSkip::GridNeedsReview
        }
    );
    // On FLAC too: the XML withholds such a grid the same way, so nothing would carry it.
    let flac = ExportSource {
        codec: Codec::Flac,
        ..wav()
    };
    assert_eq!(
        plan_with(&r, &flac, &grid_only),
        ExportOutcome::Skip {
            reason: ExportSkip::GridNeedsReview
        }
    );
    // Confirmed by ear, the grid is written.
    let plan = written(plan_decided(&r, &wav(), &grid_only, true));
    assert_eq!(plan.cut, Cut::GridOnly);
    assert!(!plan.grid_withheld);
    assert_eq!(tag(&plan, "BPM"), Some("120.00"));
}

#[test]
fn a_withheld_grid_is_flagged_on_the_written_plan() {
    let mut r = record(44_100, 101_430);
    r.tags.bpm = Some(Bpm(126.0));
    for settings in [prepare(), ExportSettings::new(BatchMode::Library)] {
        let plan = written(plan_with(&r, &wav(), &settings));
        assert!(plan.grid_withheld, "{:?}", settings.batch_mode);
        assert_eq!(tag(&plan, "BPM"), None);
    }
    // A trusted grid is not withheld, and a file without a grid has nothing to withhold.
    assert!(!written(plan_with(&record(44_100, 101_430), &wav(), &prepare())).grid_withheld);
    let mut none = record(44_100, 101_430);
    none.grid = None;
    assert!(!written(plan_with(&none, &wav(), &prepare())).grid_withheld);
}

#[test]
fn a_skipped_row_whose_grid_needs_review_still_withholds_it() {
    // Silent by the gate (no S-P95, no I) with an amber grid: decide says Skipped, which
    // outranks NeedsReview in the row's status, but the grid still needs review.
    let mut r = record(44_100, 101_430);
    r.grid.as_mut().expect("grid").confidence = Confidence::Amber;
    r.loudness.short_term_p95 = None;
    r.loudness.integrated = None;
    let plan = decide(&r, Codec::Wav, &DecideSettings::dj(), false);
    assert_eq!(plan.status, JobStage::Skipped);
    assert!(!plan.review.is_empty());
    let grid_only = ExportSettings {
        grid_only: true,
        ..prepare()
    };
    assert_eq!(
        plan_with(&r, &wav(), &grid_only),
        ExportOutcome::Skip {
            reason: ExportSkip::GridNeedsReview
        }
    );
}
