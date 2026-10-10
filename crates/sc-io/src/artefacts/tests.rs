//! Unit tests of `crates/sc-io/src/artefacts.rs`.
use super::*;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn batch_folders_are_new_and_numbered() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path().join("exports");
    let now = UNIX_EPOCH + Duration::from_secs(1_791_666_245);
    let first = new_batch_folder(&root, now).expect("created");
    let second = new_batch_folder(&root, now).expect("created");
    assert!(first.is_dir() && second.is_dir());
    let name = first.file_name().and_then(|n| n.to_str()).expect("name");
    // `yyyy-mm-dd hh.mm.ss` in the local zone.
    assert_eq!(name.len(), 19, "{name}");
    assert!(name.starts_with("2026-10-"), "{name}");
    assert_eq!(
        second.file_name().and_then(|n| n.to_str()),
        Some(format!("{name} (2)").as_str())
    );
}

#[test]
fn files_are_replaced_whole() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("soundcheck-rekordbox.xml");
    write_file(&path, b"one").expect("written");
    write_file(&path, b"two").expect("replaced");
    assert_eq!(std::fs::read(&path).expect("read"), b"two");
    // No temp file is left behind.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("list")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["soundcheck-rekordbox.xml"]);
    assert!(write_file(&dir.path().join("missing/x.csv"), b"x").is_err());
}

#[test]
fn only_our_own_artefacts_are_replaced() {
    let dir = tempfile::tempdir().expect("temp dir");
    let xml = dir.path().join("soundcheck-rekordbox.xml");
    let ours = crate::rekordbox::xml_bytes(&[], "0.0.0", "SoundCheck x");
    write_artefact(&xml, &ours, crate::rekordbox::is_soundcheck_xml).expect("new");
    write_artefact(&xml, &ours, crate::rekordbox::is_soundcheck_xml).expect("ours, replaced");
    // Someone else's XML, an audio file and a folder are never replaced.
    let theirs = dir.path().join("theirs.xml");
    let rekordbox = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n  <PRODUCT Name=\"rekordbox\" Version=\"7.2.0\" Company=\"AlphaTheta\"/>\n";
    std::fs::write(&theirs, rekordbox).expect("write");
    let wav = dir.path().join("a.wav");
    std::fs::write(&wav, b"RIFF....WAVEfmt ").expect("write");
    let folder = dir.path().join("folder.xml");
    std::fs::create_dir(&folder).expect("folder");
    for path in [&theirs, &wav, &folder] {
        let before = std::fs::symlink_metadata(path).expect("there");
        assert!(matches!(
            write_artefact(path, &ours, crate::rekordbox::is_soundcheck_xml),
            Err(Error::AlreadyExists { .. })
        ));
        assert_eq!(
            std::fs::symlink_metadata(path).expect("there").len(),
            before.len()
        );
    }
    assert_eq!(std::fs::read(&theirs).expect("read"), rekordbox);
    // The report: ours is recognised by its byte-order mark and header row.
    let csv = dir.path().join("grid-report.csv");
    let report = crate::report::csv_bytes(&[]);
    write_artefact(&csv, &report, crate::report::is_soundcheck_report).expect("new");
    write_artefact(&csv, &report, crate::report::is_soundcheck_report).expect("replaced");
    std::fs::write(&csv, b"file,mode\r\n").expect("write");
    assert!(write_artefact(&csv, &report, crate::report::is_soundcheck_report).is_err());
}
