//! Unit tests of `logging.rs`: the folder and the append-only file.

use std::io::Write;

use super::*;

#[test]
fn the_log_is_created_and_appended_to() {
    let dir = tempfile::tempdir().expect("temp dir");
    let logs = dir.path().join("Logs/app");
    let mut f = open_log(&logs).expect("opened");
    writeln!(f, "first").expect("write");
    drop(f);
    let mut f = open_log(&logs).expect("opened again");
    writeln!(f, "second").expect("write");
    drop(f);
    let text = std::fs::read_to_string(logs.join(LOG_FILE)).expect("read");
    assert_eq!(text, "first\nsecond\n");
}

#[test]
fn the_log_folder_is_named_for_the_app() {
    let dir = log_dir().expect("a home folder");
    assert!(dir.ends_with(if cfg!(target_os = "macos") {
        "Library/Logs/app.soundcheck.desktop"
    } else {
        "app.soundcheck.desktop/logs"
    }));
}
