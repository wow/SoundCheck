//! Unit tests of `crates/sc-engine/src/export/artefacts.rs`. At 44.1 kHz, 120 BPM has a
//! 22,050-sample beat; a Prepare cut leaves bar 1 at 221 samples (5 ms).
use super::*;
use sc_core::analysis::{BeatUnit, Meter};
use sc_core::export::{ExportSkip, ExportedGrid, XmlOnlyReason};
use sc_core::{Bpm, Seconds};

const SR: u32 = 44_100;

fn grid(bar1: u64, meter: Meter, confirmed: bool) -> XmlGrid {
    XmlGrid::Grid {
        grid: ExportedGrid {
            bar1: SampleIndex(bar1),
            first_bar_line: SampleIndex(bar1),
            bpm: Bpm(120.0),
            bpm_exact: Bpm(120.001),
            meter,
            edited: false,
            confirmed,
        },
        sample_rate: SR,
    }
}

fn plan(gain_db: f64, cut_frames: u64) -> ExportPlan {
    ExportPlan {
        gain_db,
        trim_frames: cut_frames,
        trim_snapped_from_frames: Some(cut_frames),
        expect_frames: 1_000_000,
        bits: None,
        tags: Vec::new(),
        bext: None,
        cut: if cut_frames > 0 {
            Cut::Cut {
                frames: cut_frames,
                seconds: SampleIndex(cut_frames).to_seconds(SR),
            }
        } else {
            Cut::Library
        },
        notices: Vec::new(),
        grid_withheld: false,
    }
}

fn written(name: &str, mode: BatchMode, g: XmlGrid) -> BatchRow {
    let path = PathBuf::from(format!("/nowhere/{name}"));
    BatchRow::written(&path, mode, &plan(-2.0, 9_261), &path, 4_410_000, SR, g)
}

fn mp3(mode: BatchMode, g: XmlGrid) -> BatchRow {
    BatchRow::not_written(
        Path::new("/nowhere/c.mp3"),
        mode,
        &ExportOutcome::XmlOnly {
            reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 },
        },
        Seconds(61.5),
        Codec::Mp3,
        g,
    )
}

fn batch(mode: BatchMode) -> Vec<BatchRow> {
    vec![
        written("a.wav", mode, grid(221, Meter::four_four(), false)),
        written("b.aiff", mode, XmlGrid::NeedsReview),
        mp3(mode, grid(44_200, Meter::four_four(), true)),
        BatchRow::not_written(
            Path::new("/nowhere/d.wav"),
            mode,
            &ExportOutcome::Skip {
                reason: ExportSkip::Silent,
            },
            Seconds(5.0),
            Codec::Wav,
            XmlGrid::Absent,
        ),
        BatchRow::failed(Path::new("/nowhere/e.wav"), Some(mode)),
    ]
}

#[test]
fn prepare_lists_written_and_xml_only_rows() {
    let rows = batch(BatchMode::Prepare);
    let tracks = xml_tracks(&rows, XmlSelect::Batch);
    let names: Vec<_> = tracks
        .iter()
        .map(|t| t.path.file_name().and_then(|n| n.to_str()).expect("name"))
        .collect();
    assert_eq!(names, ["a.wav", "b.aiff", "c.mp3"]);
    // Bar 1 at the lead after the cut: the first beat, beat 1, at 5 ms.
    let a = tracks[0].tempo.expect("a tempo");
    assert_eq!(
        (format!("{:.3}", a.inizio.0), a.battito),
        ("0.005".into(), 1)
    );
    assert!((tracks[0].duration.0 - 100.0).abs() < 1e-9);
    // Withheld: listed without a tempo.
    assert_eq!(tracks[1].tempo, None);
    // MP3, bar 1 two beats and 100 samples in: the first beat is beat 3, at 2 ms.
    assert_eq!(tracks[2].tempo.map(|t| t.battito), Some(3));
    assert!(tracks.iter().all(|t| t.path.is_absolute()));
}

#[test]
fn library_lists_only_confirmed_rows() {
    let rows = batch(BatchMode::Library);
    let tracks = xml_tracks(&rows, XmlSelect::Batch);
    assert_eq!(tracks.len(), 1, "{tracks:?}");
    assert!(tracks[0].path.ends_with("c.mp3"));
    // Named one by one, every file with something to list is listed.
    assert_eq!(xml_tracks(&rows, XmlSelect::All).len(), 3);
    assert_eq!(xml_tracks(&rows, XmlSelect::Off).len(), 0);
}

#[test]
fn report_rows_say_what_the_xml_carries() {
    let mut rows = batch(BatchMode::Prepare);
    let seven = Meter::new(BeatUnit::Eighth, &[2, 2, 3]);
    rows.push(written(
        "f.wav",
        BatchMode::Prepare,
        grid(221, seven, false),
    ));
    rows[3].notes.push("skipped: silent".into());
    let report = report_rows(&rows, XmlSelect::Batch);
    let grid: Vec<&str> = report.iter().map(|r| r.grid.as_str()).collect();
    assert_eq!(
        grid,
        [
            "tempo: beat 1 at 0.005 s",
            "withheld: grid needs review",
            "tempo: beat 3 at 0.002 s (unverified: MP3/AAC encoder delay)",
            "not in the XML",
            "not in the XML",
            "withheld: meter 7/8 · 2+2+3 (the XML carries 4/4 grids only)",
        ]
    );
    let a = &report[0];
    assert_eq!((a.mode.as_str(), a.action.as_str()), ("prepare", "written"));
    assert_eq!(a.gain_db, Some(-2.0));
    assert!((a.cut.expect("cut").0 - 0.21).abs() < 1e-12);
    assert_eq!(a.bpm, Some(Bpm(120.0)));
    assert!((a.bar1.expect("bar 1").0 - 221.0 / 44_100.0).abs() < 1e-12);
    assert_eq!((report[1].bpm, report[1].bar1), (None, None));
    assert_eq!(report[2].action, "xml only");
    assert_eq!(report[3].notes, ["skipped: silent"]);
    assert_eq!(report[4].action, "failed");
    let library = report_rows(&batch(BatchMode::Library), XmlSelect::Batch);
    assert_eq!(
        library[0].grid,
        "not in the XML: Library rows need a confirmed grid"
    );
}

#[test]
fn artefacts_are_written_together() {
    let dir = tempfile::tempdir().expect("temp dir");
    let rows = batch(BatchMode::Prepare);
    let written = write_artefacts(dir.path(), &rows, XmlSelect::Batch).expect("written");
    assert_eq!((written.listed, written.with_tempo), (3, 2));
    let xml = std::fs::read(written.xml.as_ref().expect("xml")).expect("read");
    assert_eq!(
        xml,
        sc_io::rekordbox::xml_bytes(&xml_tracks(&rows, XmlSelect::Batch), sc_core::VERSION)
    );
    let csv = std::fs::read(&written.report).expect("read");
    assert!(csv.starts_with(b"\xEF\xBB\xBFfile,mode,action,"));
    assert_eq!(csv.windows(2).filter(|w| w == b"\r\n").count(), 6);
    // Run again: the same bytes replace them.
    let again = write_artefacts(dir.path(), &rows, XmlSelect::Batch).expect("written");
    assert_eq!(std::fs::read(again.xml.expect("xml")).expect("read"), xml);
    let off = write_artefacts(dir.path(), &rows, XmlSelect::Off).expect("written");
    assert_eq!((off.xml, off.listed), (None, 0));
}
