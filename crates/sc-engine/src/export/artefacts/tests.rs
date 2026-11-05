//! Unit tests of `crates/sc-engine/src/export/artefacts.rs`. At 44.1 kHz, 120 BPM has a
//! 22,050-sample beat; a Prepare cut leaves bar 1 at 221 samples (5 ms).
use super::*;
use sc_core::analysis::{BeatUnit, Meter};
use sc_core::export::{ExportSkip, ExportedGrid, XmlOnlyReason, XmlTrackInfo};
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
            detector_offset_ms: None,
            detector_bpm: None,
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

fn info(grid: XmlGrid) -> XmlTrackInfo {
    XmlTrackInfo {
        grid,
        title: None,
        artist: None,
    }
}

fn written(name: &str, mode: BatchMode, g: XmlGrid) -> BatchRow {
    let path = PathBuf::from(format!("/nowhere/{name}"));
    BatchRow::written(
        &path,
        mode,
        &plan(-2.0, 9_261),
        &path,
        4_410_000,
        SR,
        info(g),
    )
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
        XmlTrackInfo {
            title: Some("Gece Yarısı".into()),
            artist: Some("Sezen Aksu".into()),
            ..info(g)
        },
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
            info(XmlGrid::Absent),
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
    // The withheld grid (b.aiff) is not listed: a track without a TEMPO could only change its
    // information in rekordbox.
    assert_eq!(names, ["a.wav", "c.mp3"]);
    // Bar 1 at the lead after the cut: the first beat, beat 1, at 5 ms.
    let a = tracks[0].tempo;
    assert_eq!(
        (format!("{:.3}", a.inizio.0), a.battito),
        ("0.005".into(), 1)
    );
    assert!((tracks[0].duration.0 - 100.0).abs() < 1e-9);
    assert_eq!((&tracks[0].title, &tracks[0].artist), (&None, &None));
    // MP3, bar 1 two beats and 100 samples in: the first beat is beat 3, at 2 ms; named by
    // its tags.
    assert_eq!(tracks[1].tempo.battito, 3);
    assert_eq!(tracks[1].title.as_deref(), Some("Gece Yarısı"));
    assert_eq!(tracks[1].artist.as_deref(), Some("Sezen Aksu"));
    assert!(tracks.iter().all(|t| t.path.is_absolute()));
}

#[test]
fn library_lists_only_confirmed_rows() {
    let rows = batch(BatchMode::Library);
    let tracks = xml_tracks(&rows, XmlSelect::Batch);
    assert_eq!(tracks.len(), 1, "{tracks:?}");
    assert!(tracks[0].path.ends_with("c.mp3"));
    // Named one by one, every file with a grid to list is listed, Library or not.
    assert_eq!(xml_tracks(&rows, XmlSelect::All).len(), 2);
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
            "not in the XML: grid needs review",
            "tempo: beat 3 at 0.002 s (unverified: MP3/AAC encoder delay)",
            "not in the XML",
            "not in the XML",
            "not in the XML: meter 7/8 · 2+2+3 (the XML carries 4/4 grids only)",
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
    let written =
        write_artefacts(dir.path(), &rows, XmlSelect::Batch, "SoundCheck t").expect("written");
    assert_eq!(written.listed, 2);
    let xml = std::fs::read(written.xml.as_ref().expect("xml")).expect("read");
    assert_eq!(
        xml,
        sc_io::rekordbox::xml_bytes(
            &xml_tracks(&rows, XmlSelect::Batch),
            sc_core::VERSION,
            "SoundCheck t"
        )
    );
    let csv = std::fs::read(&written.report).expect("read");
    assert!(csv.starts_with(b"\xEF\xBB\xBFfile,mode,action,"));
    assert_eq!(csv.windows(2).filter(|w| w == b"\r\n").count(), 6);
    // Run again: the same bytes replace them.
    let again =
        write_artefacts(dir.path(), &rows, XmlSelect::Batch, "SoundCheck t").expect("written");
    assert_eq!(std::fs::read(again.xml.expect("xml")).expect("read"), xml);
    let off = write_artefacts(dir.path(), &rows, XmlSelect::Off, "SoundCheck t").expect("written");
    assert_eq!((off.xml, off.listed), (None, 0));
}

/// A row of a file in `dir` this batch wrote, with a grid.
fn written_in(dir: &Path, name: &str) -> BatchRow {
    let copy = dir.join(name);
    std::fs::write(&copy, b"audio").expect("written");
    BatchRow::written(
        Path::new(&format!("/source/{name}")),
        BatchMode::Prepare,
        &plan(-1.0, 0),
        &copy,
        44_100,
        SR,
        info(grid(221, Meter::four_four(), false)),
    )
}

fn listed(dir: &Path) -> Vec<String> {
    let xml = std::fs::read(dir.join(sc_io::rekordbox::XML_FILE_NAME)).expect("xml");
    sc_io::rekordbox::read_tracks(&xml)
        .expect("our XML")
        .iter()
        .map(|t| {
            t.path
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn report_files(dir: &Path) -> Vec<String> {
    let csv = std::fs::read(dir.join(sc_io::report::REPORT_FILE_NAME)).expect("report");
    sc_io::report::read_rows(&csv)
        .expect("our report")
        .into_iter()
        .map(|r| r.file)
        .collect()
}

#[test]
fn a_second_batch_into_one_folder_keeps_what_the_first_listed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (a, b) = (
        written_in(dir.path(), "a.wav"),
        written_in(dir.path(), "b.wav"),
    );
    write_artefacts(dir.path(), &[b], XmlSelect::Batch, "SoundCheck first").expect("written");
    let second =
        write_artefacts(dir.path(), &[a], XmlSelect::Batch, "SoundCheck second").expect("written");
    assert_eq!(second.listed, 2, "both tracks are in the folder");
    assert_eq!(listed(dir.path()), ["a.wav", "b.wav"], "sorted by location");
    assert_eq!(report_files(dir.path()), ["/source/a.wav", "/source/b.wav"]);
    // Ids follow the sort, and the playlist is the batch's.
    let xml = std::fs::read_to_string(dir.path().join(sc_io::rekordbox::XML_FILE_NAME)).unwrap();
    let a_at = xml.find("a.wav").unwrap();
    assert!(xml[..a_at].contains(r#"TrackID="1""#), "{xml}");
    assert!(xml.contains(r#"Name="SoundCheck second""#), "{xml}");
    assert!(!xml.contains("SoundCheck first"), "{xml}");
}

#[test]
fn the_order_of_the_batches_does_not_change_the_files() {
    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let same = |dir: &Path| dir.join("x");
    for dir in [one.path(), two.path()] {
        std::fs::create_dir(same(dir)).unwrap();
    }
    let (a1, b1) = (
        written_in(&same(one.path()), "a.wav"),
        written_in(&same(one.path()), "b.wav"),
    );
    let (a2, b2) = (
        written_in(&same(two.path()), "a.wav"),
        written_in(&same(two.path()), "b.wav"),
    );
    for (first, second, dir) in [(a1, b1, one.path()), (b2, a2, two.path())] {
        write_artefacts(&same(dir), &[first], XmlSelect::Batch, "p").unwrap();
        write_artefacts(&same(dir), &[second], XmlSelect::Batch, "p").unwrap();
    }
    assert_eq!(listed(&same(one.path())), listed(&same(two.path())));
}

#[test]
fn what_a_kept_entry_says_survives_with_the_same_bytes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut a = written_in(dir.path(), "a, \"b\".wav");
    a.notes = vec!["=a note".to_owned(), "another".to_owned()];
    let b = written_in(dir.path(), "z.wav");
    write_artefacts(dir.path(), &[a], XmlSelect::Batch, "p").expect("written");
    let first_xml =
        std::fs::read_to_string(dir.path().join(sc_io::rekordbox::XML_FILE_NAME)).unwrap();
    let first_csv =
        std::fs::read_to_string(dir.path().join(sc_io::report::REPORT_FILE_NAME)).unwrap();
    write_artefacts(dir.path(), &[b], XmlSelect::Batch, "p").expect("written");
    let xml = std::fs::read_to_string(dir.path().join(sc_io::rekordbox::XML_FILE_NAME)).unwrap();
    let csv = std::fs::read_to_string(dir.path().join(sc_io::report::REPORT_FILE_NAME)).unwrap();
    let track_of = |text: &str| {
        let at = text.find("<TRACK ").unwrap();
        text[at..text.find("</TRACK>").unwrap()].to_owned()
    };
    assert_eq!(track_of(&xml), track_of(&first_xml));
    let row = first_csv.split("\r\n").nth(1).unwrap();
    assert!(csv.contains(row), "{row} in {csv}");
}

#[test]
fn an_entry_whose_file_is_gone_is_dropped() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (a, b) = (
        written_in(dir.path(), "a.wav"),
        written_in(dir.path(), "b.wav"),
    );
    write_artefacts(dir.path(), &[a], XmlSelect::Batch, "p").expect("written");
    std::fs::remove_file(dir.path().join("a.wav")).unwrap();
    let written = write_artefacts(dir.path(), &[b], XmlSelect::Batch, "p").expect("written");
    assert_eq!(written.listed, 1);
    assert_eq!(listed(dir.path()), ["b.wav"]);
    assert_eq!(report_files(dir.path()), ["/source/b.wav"]);
}

#[test]
fn a_track_this_batch_no_longer_lists_takes_its_old_entry_with_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let a = written_in(dir.path(), "a.wav");
    write_artefacts(dir.path(), std::slice::from_ref(&a), XmlSelect::Batch, "p").expect("written");
    let withheld = BatchRow {
        track: a.track.map(|t| RowTrack {
            grid: XmlGrid::NeedsReview,
            ..t
        }),
        ..written_in(dir.path(), "a.wav")
    };
    let written = write_artefacts(dir.path(), &[withheld], XmlSelect::Batch, "p").expect("written");
    assert_eq!(written.listed, 0);
    assert_eq!(listed(dir.path()), Vec::<String>::new());
}

#[test]
fn a_foreign_xml_there_is_neither_read_nor_replaced() {
    let dir = tempfile::tempdir().expect("temp dir");
    let foreign = "<?xml version=\"1.0\"?><DJ_PLAYLISTS/>";
    let xml = dir.path().join(sc_io::rekordbox::XML_FILE_NAME);
    std::fs::write(&xml, foreign).unwrap();
    let a = written_in(dir.path(), "a.wav");
    let refused = write_artefacts(dir.path(), &[a], XmlSelect::Batch, "p");
    assert!(
        matches!(refused, Err(sc_core::Error::AlreadyExists { .. })),
        "{refused:?}"
    );
    assert_eq!(std::fs::read_to_string(&xml).unwrap(), foreign);
}

/// Writes `a.wav`'s artefacts, damages `name` with `damage`, and returns its damaged bytes.
fn damaged(dir: &Path, name: &str, damage: fn(Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    let a = written_in(dir, "a.wav");
    write_artefacts(dir, &[a], XmlSelect::Batch, "p").expect("written");
    let path = dir.join(name);
    let bytes = damage(std::fs::read(&path).expect("read"));
    std::fs::write(&path, &bytes).expect("damaged");
    bytes
}

fn refused_and_left(dir: &Path, name: &str, bytes: &[u8]) {
    let other = std::fs::read(dir.join(if name == sc_io::report::REPORT_FILE_NAME {
        sc_io::rekordbox::XML_FILE_NAME
    } else {
        sc_io::report::REPORT_FILE_NAME
    }))
    .expect("the other file");
    let conflict = crate::export::artefact_conflict(dir).expect("refused before the job");
    assert_eq!(conflict.name, name);
    let text = conflict.error.to_string();
    assert!(
        text.contains(name) && text.contains("choose another folder"),
        "{text}"
    );

    let b = written_in(dir, "b.wav");
    let refused = write_artefacts(dir, &[b], XmlSelect::Batch, "p");
    assert!(
        matches!(refused, Err(sc_core::Error::Corrupt { .. })),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read(dir.join(name)).expect("read"),
        bytes,
        "left as it is"
    );
    // Nothing half done: the other file is not written either.
    let name_of_other = if name == sc_io::report::REPORT_FILE_NAME {
        sc_io::rekordbox::XML_FILE_NAME
    } else {
        sc_io::report::REPORT_FILE_NAME
    };
    assert_eq!(std::fs::read(dir.join(name_of_other)).expect("read"), other);
}

#[test]
fn an_xml_of_ours_cut_short_is_refused_and_left() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bytes = damaged(dir.path(), sc_io::rekordbox::XML_FILE_NAME, |b| {
        b[..b.len() - 40].to_vec()
    });
    refused_and_left(dir.path(), sc_io::rekordbox::XML_FILE_NAME, &bytes);
}

#[test]
fn an_xml_of_ours_with_a_track_without_a_tempo_is_refused_and_left() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bytes = damaged(dir.path(), sc_io::rekordbox::XML_FILE_NAME, |b| {
        String::from_utf8(b)
            .unwrap()
            .replace(
                "</COLLECTION>",
                "  <TRACK TrackID=\"9\" Name=\"x\" TotalTime=\"1\" Location=\"file://localhost/x.wav\"></TRACK>\n  </COLLECTION>",
            )
            .into_bytes()
    });
    refused_and_left(dir.path(), sc_io::rekordbox::XML_FILE_NAME, &bytes);
}

#[test]
fn a_report_of_ours_with_a_short_row_is_refused_and_left() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bytes = damaged(dir.path(), sc_io::report::REPORT_FILE_NAME, |mut b| {
        b.extend_from_slice(b"a,b\r\n");
        b
    });
    refused_and_left(dir.path(), sc_io::report::REPORT_FILE_NAME, &bytes);
}

#[test]
fn nothing_there_is_no_conflict() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(crate::export::artefact_conflict(dir.path()).is_none());
    let a = written_in(dir.path(), "a.wav");
    write_artefacts(dir.path(), &[a], XmlSelect::Batch, "p").expect("written");
    assert!(crate::export::artefact_conflict(dir.path()).is_none());
}
