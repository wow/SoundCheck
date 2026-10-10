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
