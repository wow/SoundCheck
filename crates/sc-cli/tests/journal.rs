//! `sc-cli journal`, `sc-cli journal --forget` and `sc-cli recover` end to end, with the backup
//! root in `SC_BACKUP_ROOT`. Interrupted changes are staged by writing the journal line a crash
//! would leave (the crash matrix itself, which aborts real transactions at every step, is
//! `sc-io`'s `tests/crash.rs`).

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use common::{Library, arg, wav_bytes};

/// Appends the `planned` line of an in-place change of `path` that crashed right after it,
/// with its temp file next to `path`; returns the temp file's path.
fn crashed_change(lib: &Library, txn: &str, path: &Path) -> PathBuf {
    let temp = path.with_file_name(format!(".x.soundcheck-tmp-{txn}"));
    let line = serde_json::json!({
        "txn": txn,
        "state": "planned",
        "at": "2026-10-07T10:00:00Z",
        "kind": "in_place",
        "path": path,
        "source": path,
        "temp": temp,
    });
    std::fs::create_dir_all(&lib.backups).expect("backup root");
    let mut journal = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(lib.backups.join("journal.jsonl"))
        .expect("journal");
    writeln!(journal, "{line}").expect("line");
    temp
}

#[test]
fn changes_are_listed_newest_first() {
    let lib = Library::new();
    let run = lib.run(&["journal"]);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.starts_with("no changes recorded in "),
        "{}",
        run.text()
    );

    let wav = lib.file("Track.wav", &wav_bytes(false));
    assert!(lib.run(&["apply", arg(&wav), "--gain-db", "-2"]).ok);
    assert!(lib.run(&["undo", arg(&wav)]).ok);
    let run = lib.run(&["journal"]);
    assert!(run.ok, "{}", run.text());
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", run.text());
    assert!(
        lines[0].contains("  undo  done  ") && lines[0].ends_with(arg(&wav)),
        "{}",
        run.text()
    );
    assert!(lines[1].starts_with("    from backup "), "{}", run.text());
    assert!(
        lines[2].contains("  in place  done, undone  "),
        "{}",
        run.text()
    );
    assert!(lines[3].starts_with("    backup "), "{}", run.text());

    let run = lib.run(&["journal", "--json"]);
    let doc = &run.docs()[0];
    assert_eq!(doc["schema"], 1);
    let entries = doc["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["kind"], "undo");
    assert_eq!(entries[1]["kind"], "in_place");
    assert_eq!(entries[1]["state"], "done");
    assert_eq!(entries[1]["undone"], true);
    assert!(
        entries[1]["backup"]
            .as_str()
            .is_some_and(|b| b.ends_with("Track.wav"))
    );

    let run = lib.run(&["journal", "--incomplete"]);
    assert!(
        run.stdout.starts_with("no unfinished changes recorded in "),
        "{}",
        run.text()
    );
}

#[test]
fn recover_rolls_back_an_interrupted_change() {
    let lib = Library::new();
    let wav = lib.file("Track.wav", &wav_bytes(false));
    let temp = crashed_change(&lib, "t1", &wav);
    std::fs::write(&temp, b"half a render").expect("temp");

    let run = lib.run(&["journal", "--incomplete"]);
    assert!(
        run.stdout.contains("  t1  in place  pending at planned  "),
        "{}",
        run.text()
    );
    let run = lib.run(&["recover", "--json"]);
    assert!(run.ok, "{}", run.text());
    let doc = &run.docs()[0];
    assert_eq!(doc["recovered"][0]["txn"], "t1");
    assert_eq!(doc["recovered"][0]["outcome"], "rolled_back");
    assert_eq!(doc["pending"], serde_json::json!([]));
    assert!(!temp.exists(), "the temp file is removed");
    assert_eq!(std::fs::read(&wav).expect("untouched"), wav_bytes(false));

    let run = lib.run(&["recover"]);
    assert!(
        run.stdout.starts_with("nothing to recover in "),
        "{}",
        run.text()
    );
    let run = lib.run(&["journal"]);
    assert!(
        run.stdout
            .contains("  t1  in place  recovered: rolled back  "),
        "{}",
        run.text()
    );
}

#[test]
fn apply_recovers_first() {
    let lib = Library::new();
    let wav = lib.file("Track.wav", &wav_bytes(false));
    let temp = crashed_change(&lib, "t1", &wav);
    std::fs::write(&temp, b"half a render").expect("temp");
    let run = lib.run(&["apply", arg(&wav), "--gain-db", "-1"]);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stderr
            .contains("finished or rolled back 1 change interrupted"),
        "{}",
        run.text()
    );
    assert!(!temp.exists());
}

#[test]
fn a_change_left_pending_can_be_forgotten() {
    let lib = Library::new();
    let gone = lib.base.join("Volumes/Gone/Track.wav");
    crashed_change(&lib, "t9", &gone);

    let run = lib.run(&["recover"]);
    assert!(run.ok, "{}", run.text());
    assert!(run.stdout.contains("left pending"), "{}", run.text());
    assert!(run.stdout.contains("is not reachable"), "{}", run.text());
    assert!(
        run.stdout
            .contains("to give up on it: sc-cli journal --forget t9"),
        "{}",
        run.text()
    );

    let run = lib.run(&["journal", "--forget", "t9"]);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.starts_with(&format!(
            "forgot t9 ({}, in place, stopped at planned); recovery skips it from now on\n  nothing was deleted\n",
            gone.display()
        )),
        "{}",
        run.text()
    );
    let run = lib.run(&["recover"]);
    assert!(
        run.stdout.starts_with("nothing to recover"),
        "{}",
        run.text()
    );
    let run = lib.run(&["journal"]);
    assert!(
        run.stdout
            .contains("  t9  in place  forgotten at planned  "),
        "{}",
        run.text()
    );

    let run = lib.run(&["journal", "--forget", "t9", "--json"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(run.stderr.contains("already forgotten"), "{}", run.text());
    assert_eq!(run.docs()[0]["error"]["kind"], "invalidArgument");
    let run = lib.run(&["journal", "--forget", "nope"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr.contains("no change nope is recorded"),
        "{}",
        run.text()
    );
}
