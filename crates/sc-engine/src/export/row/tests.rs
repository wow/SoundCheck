//! Unit tests of `crates/sc-engine/src/export/row.rs`: a row built from a report stays within its
//! budget whatever the file holds, and an earlier change stays revertible.

use std::path::PathBuf;

use sc_core::Seconds;
use sc_core::export::{
    Cut, ExportNotice, ExportOutcome, ExportPlan, ExportSkip, GridCheck, GridCheckSkip,
    GridDisagreement, XmlGrid, XmlTrackInfo,
};
use sc_core::ipc::{ExportRow, JobStage, MAX_ROW_BYTES};
use sc_io::render::RenderReport;
use sc_io::txn::{TxnKind, TxnReport};

use super::{done, planned, skipped, written};
use crate::edits::EditState;
use crate::export::ProcessDone;

/// Text that takes the most JSON per character.
fn long() -> String {
    "\u{1}\"".repeat(2000)
}

fn plan() -> ExportPlan {
    ExportPlan {
        gain_db: -1.5,
        trim_frames: 441,
        trim_snapped_from_frames: Some(441),
        expect_frames: 1000,
        bits: None,
        tags: Vec::new(),
        bext: None,
        cut: Cut::Cut {
            frames: 441,
            seconds: Seconds(0.01),
        },
        notices: vec![ExportNotice::SeratoCuesMayHaveShifted {
            cut_s: Seconds(0.01),
        }],
        grid_withheld: false,
    }
}

fn report(kind: TxnKind, notes: Vec<String>, stale: Vec<String>) -> TxnReport {
    TxnReport {
        txn: "6ac5b4f4-1-0".to_owned(),
        kind,
        source: PathBuf::from("/a.wav"),
        output: PathBuf::from("/a.wav"),
        backup: None,
        sidecar: None,
        render: RenderReport {
            tags_added: false,
            tags_not_added: None,
            tag_edit: None,
            vorbis_edit: None,
            frames_in: 1441,
            frames_out: 1000,
            trim_frames: 441,
            trim_requested_frames: 441,
            sample_rate_hz: 44_100,
            channels: 2,
            bits_out: 16,
            exact: false,
            dithered: false,
            samples_saturated: 0,
            pcm_hash: [0; 32],
            stale_loudness_tags: stale,
            blocks: Vec::new(),
            leading_bytes: 0,
            trailing_bytes: 0,
            output_bytes: 0,
        },
        original_blake3: [0; 32],
        output_blake3: [0; 32],
        output_bytes: 0,
        notes,
        timings: Vec::new(),
    }
}

fn process_done(notes: Vec<String>) -> ProcessDone {
    ProcessDone {
        output: PathBuf::from("/a.wav"),
        duration: Seconds(1.0),
        xml: XmlTrackInfo {
            grid: XmlGrid::Absent,
            title: None,
            artist: None,
        },
        analysis: None,
        analysed: None,
        edit: EditState::default(),
        edit_carried: false,
        grid_check: GridCheck::NotChecked {
            reason: GridCheckSkip::NotAnalysed,
        },
        notes,
    }
}

fn json_len(row: &ExportRow) -> usize {
    serde_json::to_vec(row).expect("serialisable").len()
}

#[test]
fn long_notes_and_tags_stay_within_the_budget_at_every_step() {
    let many = || vec![long(), long(), long()];
    let w = written(
        planned(&plan(), true),
        &report(TxnKind::InPlace, many(), many()),
    );
    assert_eq!(w.stage, JobStage::Writing);
    assert!(json_len(&w) <= MAX_ROW_BYTES, "written: {}", json_len(&w));
    assert_ne!(w.notes.len(), 0, "the notes are kept, cut");
    assert!(
        !w.verification
            .as_ref()
            .expect("verified")
            .stale_loudness_tags
            .is_empty(),
        "the labels are kept, cut"
    );
    let d = done(w, &process_done(many()));
    assert_eq!(d.stage, JobStage::Done);
    assert!(json_len(&d) <= MAX_ROW_BYTES, "done: {}", json_len(&d));

    // The notes after the write alone would overrun too, and join those from before it.
    let d = done(
        written(
            planned(&plan(), false),
            &report(TxnKind::InPlace, vec![], vec![]),
        ),
        &process_done(many()),
    );
    assert!(json_len(&d) <= MAX_ROW_BYTES, "done: {}", json_len(&d));
    assert_ne!(d.notes.len(), 0);
}

#[test]
fn an_earlier_change_stays_revertible_through_a_skip_or_a_copy() {
    let skip = ExportOutcome::Skip {
        reason: ExportSkip::NoGrid,
    };
    assert!(skipped(&skip, true).revertible);
    assert!(!skipped(&skip, false).revertible);
    assert!(planned(&plan(), true).revertible);
    assert!(!planned(&plan(), false).revertible);
    let copy = || report(TxnKind::ToFolder, vec![], vec![]);
    assert!(
        written(planned(&plan(), true), &copy()).revertible,
        "the earlier change stands"
    );
    assert!(
        !written(planned(&plan(), false), &copy()).revertible,
        "a copy changes nothing"
    );
    let in_place = report(TxnKind::InPlace, vec![], vec![]);
    assert!(written(planned(&plan(), false), &in_place).revertible);
}

#[test]
fn the_row_tells_the_grid_check_outcomes_apart_by_type_alone() {
    let row_with = |grid_check: GridCheck| {
        let written = written(
            planned(&plan(), true),
            &report(TxnKind::InPlace, vec![], vec![]),
        );
        let row = done(
            written,
            &ProcessDone {
                grid_check,
                ..process_done(vec![])
            },
        );
        serde_json::to_value(&row).expect("serialisable")["gridCheck"].clone()
    };
    let advisory = row_with(GridCheck::Advisory {
        disagreement: GridDisagreement::OffBy {
            offset_ms: -416.7,
            bpm_diff: 0.0,
        },
    });
    assert_eq!(advisory["result"], "advisory");
    assert_eq!(advisory["disagreement"]["kind"], "offBy");
    assert_eq!(advisory["disagreement"]["offsetMs"], -416.7);
    let check_bar_1 = row_with(GridCheck::CheckBar1 {
        offset_ms: -416.7,
        bpm_diff: 0.0,
        beats: -1,
    });
    assert_eq!(check_bar_1["result"], "checkBar1");
    assert_eq!(check_bar_1["beats"], -1);
    let reported = row_with(GridCheck::OffBy {
        offset_ms: 7.2,
        bpm_diff: 0.0,
    });
    assert_eq!(reported["result"], "offBy");
    assert_eq!(reported["offsetMs"], 7.2);
}

#[test]
fn a_skipped_row_says_why_in_the_same_words_as_the_report() {
    use crate::export::wording::{skip_why, xml_only_text};
    use sc_core::export::XmlOnlyReason;
    use sc_core::plan::Codec;

    let reason = ExportSkip::NotDjSafeRate {
        sample_rate_hz: 96_000,
    };
    let row = skipped(&ExportOutcome::Skip { reason }, false);
    assert_eq!(row.notes, [skip_why(reason)]);

    let reason = XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 };
    let row = skipped(&ExportOutcome::XmlOnly { reason }, false);
    assert_eq!(row.notes, [xml_only_text(reason)]);
}

#[test]
fn what_the_plan_says_is_worded_and_kept_through_every_step() {
    use crate::export::wording::{GRID_WITHHELD, notice_text};

    let plan = ExportPlan {
        grid_withheld: true,
        ..plan()
    };
    let notice = notice_text(plan.notices[0]);
    let row = planned(&plan, false);
    assert_eq!(row.notes, [notice.clone(), GRID_WITHHELD.to_owned()]);

    let row = written(
        row,
        &report(TxnKind::InPlace, vec!["a note".to_owned()], Vec::new()),
    );
    assert_eq!(
        row.notes,
        [
            notice.clone(),
            GRID_WITHHELD.to_owned(),
            "a note".to_owned()
        ]
    );

    let row = done(row, &process_done(vec!["after".to_owned()]));
    assert_eq!(
        row.notes,
        [
            notice,
            GRID_WITHHELD.to_owned(),
            "a note".to_owned(),
            "after".to_owned()
        ]
    );
}
