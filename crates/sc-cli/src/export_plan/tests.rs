//! Unit tests of `crates/sc-cli/src/export_plan.rs`: the text of every outcome.
use super::*;
use sc_core::plan::Codec;
use sc_core::{SampleIndex, Seconds, Tag};

fn text(outcome: &ExportOutcome) -> String {
    let mut out = Vec::new();
    write_outcome(&mut out, outcome, BatchMode::Prepare).expect("written");
    String::from_utf8(out).expect("utf-8")
}

fn plan(cut: Cut) -> ExportPlan {
    ExportPlan {
        gain_db: -2.04,
        trim_frames: 0,
        trim_snapped_from_frames: None,
        expect_frames: 1,
        bits: None,
        tags: vec![Tag::new("BPM", "120.00"), Tag::new("SOUNDCHECK", "v=1")],
        bext: None,
        cut,
        notices: Vec::new(),
        grid_withheld: false,
    }
}

#[test]
fn written_files_name_the_gain_the_cut_and_the_tags() {
    let cut = ExportOutcome::Write {
        plan: plan(Cut::Cut {
            frames: 9261,
            seconds: Seconds(0.21),
        }),
    };
    assert_eq!(
        text(&cut),
        "  Export (prepare): Gain -2.0 dB, Cut 0.21 s; tags BPM, SOUNDCHECK\n"
    );
    let not_cut = |bar1: u64, first: u64| {
        text(&ExportOutcome::Write {
            plan: plan(Cut::NotCut {
                bar1: SampleIndex(bar1),
                bar1_s: SampleIndex(bar1).to_seconds(44_100),
                first_bar_line: SampleIndex(first),
                first_bar_line_s: SampleIndex(first).to_seconds(44_100),
            }),
        })
    };
    let t = not_cut(44_100, 44_100);
    assert!(t.contains("Gain -2.0 dB, Not cut: bar 1 1.00 s in;"), "{t}");
    let t = not_cut(396_900, 44_100);
    assert!(
        t.contains("Not cut: first bar line 1.00 s in (bar 1 at 9.00 s);"),
        "{t}"
    );
    let review = text(&ExportOutcome::Write {
        plan: plan(Cut::NeedsReview {
            bar1: SampleIndex(101_430),
            bar1_s: SampleIndex(101_430).to_seconds(44_100),
        }),
    });
    assert!(
        review.contains(
            "Gain -2.0 dB, Not cut: grid needs review (bar 1 at 2.30 s; confirm it in the app);"
        ),
        "{review}"
    );
    let mut grid_only = plan(Cut::GridOnly);
    grid_only.gain_db = 0.0;
    assert!(
        text(&ExportOutcome::Write { plan: grid_only }).contains("Grid only (no audio change)")
    );
    let mut library = plan(Cut::Library);
    library.bits = Some(16);
    assert!(text(&ExportOutcome::Write { plan: library }).contains("length kept, 16-bit"));
}

#[test]
fn xml_only_names_the_reason() {
    let mp3 = ExportOutcome::XmlOnly {
        reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 },
    };
    assert_eq!(
        text(&mp3),
        "  Export (prepare): XML only: MP3 (file writes arrive later)\n"
    );
    for reason in [
        XmlOnlyReason::GridOnlyFlac,
        XmlOnlyReason::GridOnlyWouldRequantise {
            float: true,
            bits: Some(32),
        },
        XmlOnlyReason::NoTagToWriteGridOnly,
    ] {
        let t = text(&ExportOutcome::XmlOnly { reason });
        assert!(t.contains("XML only: grid only"), "{t}");
    }
}

#[test]
fn every_skip_has_three_lines() {
    for reason in [
        ExportSkip::SeratoInPlaceCut {
            cut_s: Seconds(0.29),
        },
        ExportSkip::SeratoUnknownInPlaceCut {
            cut_s: Seconds(0.29),
        },
        ExportSkip::NotDjSafeRate {
            sample_rate_hz: 96_000,
        },
        ExportSkip::Unsupported { codec: Codec::Alac },
        ExportSkip::Silent,
        ExportSkip::NoGrid,
        ExportSkip::GridNeedsReview,
        ExportSkip::UnsupportedChannels { channels: 6 },
        ExportSkip::NothingToWrite {
            reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 },
        },
    ] {
        let t = text(&ExportOutcome::Skip { reason });
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 3, "{t}");
        assert_eq!(lines[0], "  Export (prepare): skipped");
        assert!(
            lines[1].starts_with("    why: ") && lines[1].len() > 20,
            "{t}"
        );
        assert!(
            lines[2].starts_with("    what to do: ") && lines[2].len() > 25,
            "{t}"
        );
    }
}

#[test]
fn unreadable_tags_say_serato_could_not_be_ruled_out() {
    let t = text(&ExportOutcome::Skip {
        reason: ExportSkip::SeratoUnknownInPlaceCut {
            cut_s: Seconds(0.29),
        },
    });
    assert!(
        t.contains("why: its tags could not be read to rule out Serato data, and cutting 0.29 s"),
        "{t}"
    );
    assert!(t.contains("what to do: export it to a folder"), "{t}");
}

#[test]
fn on_bar_wording() {
    let on_bar = |line: u64, bar1: u64| {
        text(&ExportOutcome::Write {
            plan: plan(Cut::OnBar {
                bar_line: SampleIndex(line),
                bar_line_s: SampleIndex(line).to_seconds(44_100),
                bar1: SampleIndex(bar1),
                bar1_s: SampleIndex(bar1).to_seconds(44_100),
            }),
        })
    };
    assert_eq!(
        on_bar(0, 0),
        "  Export (prepare): Gain -2.0 dB, Starts on bar 1; tags BPM, SOUNDCHECK\n"
    );
    let t = on_bar(132, 132);
    assert!(
        t.contains("Gain -2.0 dB, Starts on bar 1, 0.003 s in;"),
        "{t}"
    );
    // Bar 1 at 8.000 s at 120 BPM: the file starts on the bar line four bars before it.
    let t = on_bar(0, 352_800);
    assert!(
        t.contains("Gain -2.0 dB, Starts on a bar line (bar 1 at 8.00 s);"),
        "{t}"
    );
    let t = on_bar(132, 352_932);
    assert!(
        t.contains("Starts on a bar line 0.003 s in (bar 1 at 8.00 s);"),
        "{t}"
    );
}

#[test]
fn notices_follow_the_line() {
    let mut p = plan(Cut::Cut {
        frames: 12_789,
        seconds: Seconds(0.29),
    });
    p.notices = vec![ExportNotice::SeratoCuesShifted {
        cut_s: Seconds(0.29),
    }];
    let t = text(&ExportOutcome::Write { plan: p });
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines.len(), 2, "{t}");
    assert!(
        lines[1].starts_with("    note: the copy keeps its Serato cue points")
            && lines[1].contains("0.29 s late"),
        "{t}"
    );
}

#[test]
fn a_withheld_grid_is_noted() {
    let mut p = plan(Cut::Library);
    p.grid_withheld = true;
    let t = text(&ExportOutcome::Write { plan: p });
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines.len(), 2, "{t}");
    assert_eq!(lines[1], format!("    note: {GRID_WITHHELD}"));
    let t = text(&ExportOutcome::Skip {
        reason: ExportSkip::GridNeedsReview,
    });
    assert!(
        t.contains("why: grid only, and its grid needs review"),
        "{t}"
    );
}
