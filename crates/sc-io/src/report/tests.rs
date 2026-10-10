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
    // An empty report is the header alone.
    assert_eq!(text(&[]), format!("{}\r\n", COLUMNS.join(",")));
}
