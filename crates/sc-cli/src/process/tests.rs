//! Unit tests of `crates/sc-cli/src/process.rs`: how a file's grid check reads in the text and
//! JSON output. Which result a file gets depends on the beat model's reading of its audio, so the
//! results are made here rather than provoked.

use sc_core::analysis::Meter;
use sc_core::export::{Cut, GridDisagreement, XmlGrid};
use sc_engine::EditState;
use sc_io::render::RenderReport;

use super::*;

fn plan() -> ExportPlan {
    ExportPlan {
        gain_db: -3.9,
        trim_frames: 0,
        trim_snapped_from_frames: None,
        expect_frames: 1000,
        bits: None,
        tags: Vec::new(),
        bext: None,
        cut: Cut::Library,
        notices: Vec::new(),
        grid_withheld: false,
    }
}

fn report() -> TxnReport {
    TxnReport {
        txn: "6ac5b4f4-1-0".to_owned(),
        kind: TxnKind::InPlace,
        source: PathBuf::from("/a.wav"),
        output: PathBuf::from("/a.wav"),
        backup: None,
        sidecar: None,
        render: RenderReport {
            tags_added: false,
            tags_not_added: None,
            tag_edit: None,
            vorbis_edit: None,
            frames_in: 1000,
            frames_out: 1000,
            trim_frames: 0,
            trim_requested_frames: 0,
            sample_rate_hz: 44_100,
            channels: 2,
            bits_out: 16,
            exact: false,
            dithered: false,
            samples_saturated: 0,
            pcm_hash: [0; 32],
            stale_loudness_tags: Vec::new(),
            blocks: Vec::new(),
            leading_bytes: 0,
            trailing_bytes: 0,
            output_bytes: 0,
        },
        original_blake3: [0; 32],
        output_blake3: [0; 32],
        output_bytes: 0,
        notes: Vec::new(),
        timings: Vec::new(),
    }
}

fn written_with(grid_check: GridCheck) -> FileResult {
    FileResult::Written {
        plan: Box::new(plan()),
        report: Box::new(report()),
        done: Box::new(ProcessDone {
            output: PathBuf::from("/a.wav"),
            duration: Seconds(1.0),
            xml: XmlTrackInfo {
                grid: XmlGrid::Absent,
                title: None,
                artist: None,
            },
            analysis: None,
            edit: EditState::default(),
            edit_carried: false,
            grid_check,
            notes: Vec::new(),
        }),
    }
}

/// The `  grid check:` line and the JSON `written.gridCheck` of a file with this result.
fn outputs(check: &GridCheck) -> (String, serde_json::Value) {
    let file = Path::new("/a.wav");
    let result = written_with(check.clone());
    let mut text = Vec::new();
    write_lines(&mut text, file, &result, BatchMode::Library).expect("written");
    let text = String::from_utf8(text).expect("UTF-8");
    let line = text
        .lines()
        .nth(1)
        .and_then(|l| l.strip_prefix("  grid check: "))
        .unwrap_or_else(|| panic!("no grid check line: {text}"))
        .to_owned();
    let doc = doc(file, &result).expect("a document");
    let json = serde_json::to_value(&doc).expect("serialises");
    (line, json["written"]["gridCheck"].clone())
}

#[test]
fn a_disagreement_on_a_confirmed_grid_prints_as_advisory_with_its_numbers() {
    let (line, json) = outputs(&GridCheck::Advisory {
        disagreement: GridDisagreement::OffBy {
            offset_ms: -416.74,
            bpm_diff: 0.0,
        },
    });
    assert_eq!(
        line,
        "the analysis reads this differently; your confirmed grid was written: \
         off by -416.7 ms (BPM +0.0000)"
    );
    assert_eq!(
        json,
        serde_json::json!({
            "result": "advisory",
            "disagreement": {"kind": "offBy", "offsetMs": -416.74, "bpmDiff": 0.0}
        })
    );
    let (line, json) = outputs(&GridCheck::Advisory {
        disagreement: GridDisagreement::MeterDiffers {
            found: Meter::three_four(),
        },
    });
    assert!(
        line.ends_with("written: meter differs (found 3/4)"),
        "{line}"
    );
    assert_eq!(json["disagreement"]["kind"], "meterDiffers");
    assert_eq!(json["disagreement"]["found"]["beatsPerBar"], 3);
    let (line, json) = outputs(&GridCheck::Advisory {
        disagreement: GridDisagreement::NoGridFound,
    });
    assert!(
        line.ends_with("written: no grid found in the written file"),
        "{line}"
    );
    assert_eq!(json["disagreement"]["kind"], "noGridFound");
}

#[test]
fn a_whole_beat_on_an_unconfirmed_grid_prints_as_check_bar_1() {
    let (line, json) = outputs(&GridCheck::CheckBar1 {
        offset_ms: -416.74,
        bpm_diff: 0.000_2,
        beats: -1,
    });
    assert_eq!(
        line,
        "check bar 1: the analysis reads it 1 beat earlier (bar line -416.7 ms, BPM +0.0002)"
    );
    assert_eq!(
        json,
        serde_json::json!({
            "result": "checkBar1", "offsetMs": -416.74, "bpmDiff": 0.000_2, "beats": -1
        })
    );
}

#[test]
fn another_disagreement_on_an_unconfirmed_grid_prints_as_before() {
    let (line, json) = outputs(&GridCheck::OffBy {
        offset_ms: 7.25,
        bpm_diff: 0.0,
    });
    assert_eq!(line, "off by +7.2 ms (BPM +0.0000)");
    assert_eq!(json["result"], "offBy");
}
