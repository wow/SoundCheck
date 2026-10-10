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
    // 86,400, and the last beat (the group of three) 28,800.
    let mut r = record(48_000, 240 + 86_400 + 28_800);
    let grid = r.grid.as_mut().expect("grid");
    grid.meter = Meter::new(BeatUnit::Eighth, &[2, 2, 2, 3]);
    grid.bpm = Bpm(300.0);
    let bars = Bars::of(grid, 48_000).expect("bars");
    assert!((bars.bar - 86_400.0).abs() < 1e-9 && (bars.last_beat - 28_800.0).abs() < 1e-9);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert_eq!(plan.trim_frames, 28_800);
    assert_eq!(tag(&plan, "BPM"), Some("300.00"));
    // With the long group first, the last beat is two pulses: the same cut is too long.
    r.grid.as_mut().expect("grid").meter = Meter::new(BeatUnit::Eighth, &[3, 2, 2, 2]);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert!(matches!(plan.cut, Cut::NotCut { .. }), "{:?}", plan.cut);
}
