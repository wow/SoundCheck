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
        expect_frames: 1,
        bits: None,
        tags: vec![Tag::new("BPM", "120.00"), Tag::new("SOUNDCHECK", "v=1")],
        bext: None,
        cut,
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
    let not_cut = ExportOutcome::Write {
        plan: plan(Cut::NotCut {
            first_bar_line: SampleIndex(352_800),
            first_bar_line_s: Seconds(8.0),
        }),
    };
    assert!(
        text(&not_cut).contains("Not cut: first bar line 8.00 s in"),
        "{}",
        text(&not_cut)
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
        ExportSkip::NotDjSafeRate {
            sample_rate_hz: 96_000,
        },
        ExportSkip::Unsupported { codec: Codec::Alac },
        ExportSkip::Silent,
        ExportSkip::NoGrid,
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
