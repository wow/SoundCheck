//! How the grid check's outcomes (`sc_core::export::GridCheck`) are recorded and read, without the
//! model: which outcome a file gets depends on the beat model's reading of its audio, so the
//! outcomes are made here rather than provoked. Each reaches the grid report and the sidecar with
//! its numbers; a disagreement an earlier version recorded on a confirmed grid reads as advisory;
//! a later version's result reads as absent.

mod common;

use std::path::{Path, PathBuf};

use common::export::*;
use sc_core::Seconds;
use sc_core::export::{
    BatchMode, ExportOutcome, GridCheck, GridDisagreement, XmlGrid, XmlOnlyReason, XmlTrackInfo,
};
use sc_core::plan::Codec;
use sc_engine::export::report_rows;
use sc_engine::{BatchRow, CancelToken, XmlSelect};

/// One row of a report holding `check`, as `sc-cli process` writes it, its bytes as text.
fn report_with(check: &GridCheck) -> String {
    let outcome = ExportOutcome::XmlOnly {
        reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 },
    };
    let mut row = BatchRow::not_written(
        Path::new("/m/a.mp3"),
        BatchMode::Prepare,
        &outcome,
        Seconds(200.0),
        Codec::Mp3,
        XmlTrackInfo {
            grid: XmlGrid::Absent,
            title: None,
            artist: None,
        },
    );
    row.grid_check = Some(check.clone());
    let cells = report_rows(&[row], XmlSelect::Batch);
    let mut bytes = Vec::new();
    sc_io::report::write_csv(&cells, &mut bytes).expect("written");
    String::from_utf8(bytes).expect("UTF-8")
}

#[test]
fn the_grid_report_cell_keeps_the_numbers_of_every_outcome() {
    let off = GridCheck::OffBy {
        offset_ms: -416.74,
        bpm_diff: 0.000_2,
    };
    let report = report_with(&off.clone().on_confirmed_grid());
    assert!(
        report.contains(
            ",the analysis reads this differently; your confirmed grid was kept: \
             off by -416.7 ms (BPM +0.0002),"
        ),
        "{report}"
    );
    let report = report_with(&GridCheck::CheckBar1 {
        offset_ms: -416.74,
        bpm_diff: 0.000_2,
        beats: -1,
    });
    // Quoted: the wording holds a comma.
    assert!(
        report.contains(
            r#","check bar 1: the analysis reads it 1 beat earlier (bar line -416.7 ms, BPM +0.0002)""#
        ),
        "{report}"
    );
    let report = report_with(&off);
    assert!(
        report.contains(",off by -416.7 ms (BPM +0.0002),"),
        "{report}"
    );
    let bpm = GridCheck::BpmDiffers {
        bpm_diff: 0.006,
        found: sc_core::Bpm(120.006),
    };
    let report = report_with(&bpm.on_confirmed_grid());
    assert!(
        report.contains(": BPM differs by +0.0060 (found 120.006),"),
        "{report}"
    );
    let meter = GridCheck::MeterDiffers {
        found: sc_core::analysis::Meter::three_four(),
        exported: Some(sc_core::analysis::Meter::four_four()),
    };
    // Both meters, in either state; quoted, as the wording holds a comma.
    let report = report_with(&meter);
    assert!(
        report.contains(r#","meter differs (found 3/4, exported 4/4)""#),
        "{report}"
    );
    let report = report_with(&meter.on_confirmed_grid());
    assert!(
        report.contains(": meter differs (found 3/4, exported 4/4)\""),
        "{report}"
    );
}

/// A tone exported in Library mode: its path, and its sidecar as JSON.
fn exported_tone(lib: &Lib) -> (PathBuf, serde_json::Value) {
    let path = common::tone_wav(&lib.music(), "tone.wav", 12.0, 0.9);
    let s = settings(
        lib,
        common::loudness_only(),
        process(lib, BatchMode::Library, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let sidecar = sc_io::txn::sidecar_path(&path);
    let value = serde_json::from_slice(&std::fs::read(sidecar).expect("sidecar")).expect("JSON");
    (path, value)
}

/// The sidecar `value` with a grid confirmed or not, and the check it records.
fn with_check(
    mut value: serde_json::Value,
    confirmed: bool,
    check: &serde_json::Value,
) -> serde_json::Value {
    value["export"]["grid"] = serde_json::json!({
        "bar1": 220,
        "firstBarLine": 220,
        "bpm": 120.0,
        "bpmExact": 120.0,
        "meter": sc_core::analysis::Meter::four_four(),
        "edited": false,
        "confirmed": confirmed,
    });
    value["export"]["gridCheck"] = check.clone();
    value
}

#[test]
fn the_new_outcomes_round_trip_through_the_sidecar_with_their_numbers() {
    let lib = Lib::new();
    let (path, value) = exported_tone(&lib);
    let txn = value["transaction"]
        .as_str()
        .expect("transaction")
        .to_owned();
    let sidecar = sc_io::txn::sidecar_path(&path);
    for (confirmed, check) in [
        (
            false,
            GridCheck::CheckBar1 {
                offset_ms: -416.74,
                bpm_diff: 0.000_2,
                beats: -1,
            },
        ),
        (
            true,
            GridCheck::OffBy {
                offset_ms: -416.74,
                bpm_diff: 0.000_2,
            }
            .on_confirmed_grid(),
        ),
        (true, GridCheck::NoGridFound.on_confirmed_grid()),
    ] {
        let first = with_check(value.clone(), confirmed, &serde_json::Value::Null);
        std::fs::write(&sidecar, serde_json::to_vec(&first).expect("JSON")).expect("written");
        sc_io::txn::record_grid_check(&lib.backups(), &txn, &path, &check).expect("recorded");
        let read = sc_io::txn::sidecar::read(&sidecar).expect("reads");
        assert_eq!(read.export.expect("export").grid_check, Some(check));
    }
}

#[test]
fn a_disagreement_recorded_earlier_on_a_confirmed_grid_reads_as_advisory() {
    let lib = Lib::new();
    let (path, value) = exported_tone(&lib);
    let sidecar = sc_io::txn::sidecar_path(&path);
    let recorded = serde_json::json!({"result": "offBy", "offsetMs": -416.7, "bpmDiff": 0.0});
    let read_with = |confirmed: bool| {
        let doc = with_check(value.clone(), confirmed, &recorded);
        std::fs::write(&sidecar, serde_json::to_vec(&doc).expect("JSON")).expect("written");
        let read = sc_io::txn::sidecar::read(&sidecar).expect("reads");
        read.export.expect("export").grid_check.expect("a result")
    };
    let confirmed = read_with(true);
    assert!(!confirmed.failed(), "{confirmed:?}");
    assert_eq!(
        confirmed,
        GridCheck::Advisory {
            disagreement: GridDisagreement::OffBy {
                offset_ms: -416.7,
                bpm_diff: 0.0,
            }
        }
    );
    // Not confirmed, it stays as it was recorded.
    assert!(read_with(false).failed());
}

#[test]
fn a_later_result_in_a_sidecar_reads_as_absent() {
    let lib = Lib::new();
    let (path, value) = exported_tone(&lib);
    let sidecar = sc_io::txn::sidecar_path(&path);
    let later = serde_json::json!({"result": "somethingLater", "offsetMs": 1.0});
    let doc = with_check(value, true, &later);
    std::fs::write(&sidecar, serde_json::to_vec(&doc).expect("JSON")).expect("written");
    let read = sc_io::txn::sidecar::read(&sidecar).expect("reads");
    assert_eq!(read.export.expect("export").grid_check, None);
}
