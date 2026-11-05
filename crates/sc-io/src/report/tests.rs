//! Unit tests of `crates/sc-io/src/report.rs`. Expected bytes are written by hand.
use super::*;

fn text(rows: &[ReportRow]) -> String {
    let bytes = csv_bytes(rows);
    let body = bytes
        .strip_prefix(BOM)
        .expect("starts with the byte-order mark");
    String::from_utf8(body.to_vec()).expect("UTF-8")
}

#[test]
fn csv_columns() {
    let rows = [
        ReportRow {
            file: "/Müzik/Şarkı, \"canlı\".wav".into(),
            mode: "prepare".into(),
            action: "written".into(),
            gain_db: Some(-2.304),
            cut: Some(Seconds(0.21)),
            bpm: Some(Bpm(127.996)),
            bar1: Some(Seconds(0.005_011)),
            grid: "tempo".into(),
            grid_check: None,
            notes: vec!["cut 0.21 s".into(), "tags BPM".into()],
        },
        ReportRow {
            file: "=HYPERLINK(1).mp3".into(),
            mode: "library".into(),
            action: "skipped".into(),
            gain_db: Some(-0.0),
            grid: "withheld: grid needs review".into(),
            notes: vec![" leading space".into()],
            ..ReportRow::default()
        },
        ReportRow {
            file: "a\nb.wav".into(),
            action: "failed".into(),
            ..ReportRow::default()
        },
    ];
    let expected = "file,mode,action,gain_db,cut_s,bpm,bar1_s,grid,grid_check,notes\r\n\
        \"/Müzik/Şarkı, \"\"canlı\"\".wav\",prepare,written,-2.30,0.210,128.00,0.005,tempo,,cut 0.21 s; tags BPM\r\n\
        '=HYPERLINK(1).mp3,library,skipped,+0.00,,,,withheld: grid needs review,,\" leading space\"\r\n\
        \"a\nb.wav\",,failed,,,,,,,\r\n";
    assert_eq!(text(&rows), expected);
    // Ten columns in the header, the BOM first, and the same bytes every time.
    assert_eq!(COLUMNS.len(), 10);
    assert!(csv_bytes(&rows).starts_with(b"\xEF\xBB\xBFfile,mode,"));
    assert_eq!(csv_bytes(&rows), csv_bytes(&rows));
    // Every text cell that a spreadsheet would run as a formula is neutralised; numbers are not.
    let risky = [ReportRow {
        file: "-1+2.flac".into(),
        mode: "@mode".into(),
        action: "+x".into(),
        gain_db: Some(-2.3),
        grid: "\tgrid".into(),
        grid_check: Some("\rcheck".into()),
        notes: vec!["@SUM(1)".into(), "fine".into()],
        ..ReportRow::default()
    }];
    assert_eq!(
        text(&risky).lines().nth(1),
        Some("'-1+2.flac,'@mode,'+x,-2.30,,,,'\tgrid,\"'\rcheck\",'@SUM(1); fine")
    );
    // An empty report is the header alone.
    assert_eq!(text(&[]), format!("{}\r\n", COLUMNS.join(",")));
}

#[test]
fn what_it_writes_reads_back_as_the_same_bytes() {
    let rows = [
        ReportRow {
            file: "/Music/a, \"b\"\nc.wav".to_owned(),
            mode: "prepare".to_owned(),
            action: "written".to_owned(),
            gain_db: Some(-2.25),
            cut: Some(Seconds(0.21)),
            bpm: Some(Bpm(128.0)),
            bar1: Some(Seconds(0.005)),
            grid: "tempo: beat 1 at 0.005 s".to_owned(),
            grid_check: Some("off by 12.0 ms".to_owned()),
            notes: vec!["=formula".to_owned(), "another".to_owned()],
        },
        ReportRow {
            file: "=b.wav".to_owned(),
            ..ReportRow::default()
        },
    ];
    let bytes = csv_bytes(&rows);
    let read = read_rows(&bytes).expect("ours");
    assert_eq!(read[0].file, rows[0].file);
    assert_eq!(read[1].file, "=b.wav");
    assert_eq!(read[1].gain_db, None);
    assert_eq!(csv_bytes(&read), bytes);
}

#[test]
fn another_shape_of_report_is_not_read() {
    assert_eq!(read_rows(b"file,mode\r\n"), None);
    assert_eq!(read_rows(b"\xEF\xBB\xBFfile,mode\r\n"), None);
}

#[test]
fn a_report_cut_short_or_damaged_is_not_an_empty_one() {
    let rows = [ReportRow {
        file: "/a, \"b\".wav".to_owned(),
        gain_db: Some(-1.0),
        ..ReportRow::default()
    }];
    let bytes = csv_bytes(&rows);
    assert!(read_rows(&bytes).is_some());
    assert_eq!(
        read_rows(&bytes[..bytes.len() - 6]),
        None,
        "a row cut short"
    );
    let quote = bytes.iter().rposition(|b| *b == b'"').unwrap();
    assert_eq!(read_rows(&bytes[..quote]), None, "cut in a quoted field");
    let mut short = bytes.clone();
    short.extend_from_slice(b"just one field\r\n");
    assert_eq!(read_rows(&short), None, "a row with too few fields");
    // Cut inside the last field, quoted: every field is there, but not all of it.
    let noted = csv_bytes(&[ReportRow {
        file: "/a.wav".to_owned(),
        notes: vec!["a, b".to_owned()],
        ..ReportRow::default()
    }]);
    assert!(read_rows(&noted).is_some());
    assert_eq!(
        read_rows(&noted[..noted.len() - 4]),
        None,
        "cut in the last field"
    );
    let text = String::from_utf8(bytes).unwrap().replace("-1.00", "oops");
    assert_eq!(read_rows(text.as_bytes()), None, "a number that is none");
}
