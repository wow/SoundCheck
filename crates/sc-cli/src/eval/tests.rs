//! Unit tests of the private parts of `crates/sc-cli/src/eval.rs`.
use super::*;

#[test]
fn labels_parse_with_quotes_and_optional_columns() {
    let text = "# a comment\nfile,bpm,bar1_s,meter,grouping,ffmpeg_i_lufs\n\
                \"Artist, The - Song.flac\",123.87,0.455,4/4,,-9.8\n\
                Aksak.flac,,,9/8,2+2+2+3,\n";
    let labels = parse_labels(text).unwrap();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].file, "Artist, The - Song.flac");
    assert_eq!(labels[0].bpm, Some(123.87));
    assert_eq!(labels[0].ffmpeg_i_lufs, Some(-9.8));
    assert_eq!(labels[1].grouping, Some(vec![2, 2, 2, 3]));
    assert_eq!(labels[1].bpm, None);
    assert_eq!(
        meter_text(labels[1].meter.as_deref(), labels[1].grouping.as_deref()).as_deref(),
        Some("9/8 · 2+2+2+3")
    );
    assert!(parse_labels("bpm\n120\n").is_err());
    assert_eq!(label_beats_per_bar(&labels[1]), Some(9.0));
    assert_eq!(label_beats_per_bar(&labels[0]), Some(4.0));
    let unlabelled = Label::default();
    assert_eq!(label_beats_per_bar(&unlabelled), None);
    let zero = Label {
        grouping: Some(vec![2, 0, 2]),
        meter: Some("6/8".into()),
        ..Label::default()
    };
    assert_eq!(
        label_beats_per_bar(&zero),
        Some(6.0),
        "a zero group is ignored"
    );
    assert!(parse_labels("file,bpm\nx.flac,fast\n").is_err());
}

#[test]
fn the_fit_column_is_optional_and_whole_by_default() {
    let labels = parse_labels(
        "file,bpm,fit\nA.flac,117.00,start\nB.flac,120,\nC.flac,121,Whole\nD.flac,122,START\n",
    )
    .unwrap();
    let fits: Vec<GridFit> = labels.iter().map(|l| l.fit).collect();
    assert_eq!(
        fits,
        [
            GridFit::Start,
            GridFit::Whole,
            GridFit::Whole,
            GridFit::Start
        ]
    );
    let old = parse_labels("file,bpm\nA.flac,117.00\n").unwrap();
    assert_eq!(
        old[0].fit,
        GridFit::Whole,
        "an old labels file has no fit column"
    );
    let err = parse_labels("file,fit\nA.flac,middle\n").unwrap_err();
    assert!(err.contains("row 2") && err.contains("middle"), "{err}");
}
