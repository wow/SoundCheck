//! The crash matrix (feature `crash-test`): the `txn_apply` program (`examples/txn_apply.rs`)
//! is run with `SC_TEST_CRASH_AFTER_STEP` naming a crash point, so it aborts there: every
//! journaled state and the points between them (inside the render, inside the backup copy,
//! between the backup's journaled name and its rename, between the rename and its journal
//! line), for in-place changes of a WAV and a FLAC, copies to a folder, and undos. Recovery
//! must then leave the file before the transaction or the verified file after it (never
//! anything else, and after it exactly when the rename happened); no temp, placeholder or lock
//! file; the journal entry recovered; for an in-place change the backup present (equal to the
//! original) exactly when the rename happened. A second recovery finds nothing, and undo
//! leaves the original.

mod common;

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{Library, assert_no_temps, blake3_of, files_under, flac, wav};
use sc_core::{Error, RenderRequest};
use sc_io::txn::{
    self, BACKUP_ROOT_ENV, CRASH_ENV, Outcome, SIDECAR_SUFFIX, State, TxnOptions, hex,
    journal_entries, sidecar_path,
};

/// SIGABRT, what `std::process::abort` raises.
const SIGABRT: i32 = 6;

/// Crash points of an in-place change and the furthest state each leaves journaled.
const IN_PLACE: [(&str, State); 10] = [
    ("planned", State::Planned),
    ("render", State::Planned),
    ("temp_written", State::TempWritten),
    ("verified", State::Verified),
    ("backup_copy", State::Verified),
    ("backup_named", State::Verified),
    ("backed_up", State::BackedUp),
    ("rename", State::BackedUp),
    ("renamed", State::Renamed),
    ("metadata_done", State::MetadataDone),
];

/// Crash points of a copy to a folder.
const TO_FOLDER: [(&str, State); 7] = [
    ("planned", State::Planned),
    ("render", State::Planned),
    ("temp_written", State::TempWritten),
    ("verified", State::Verified),
    ("rename", State::Verified),
    ("renamed", State::Renamed),
    ("metadata_done", State::MetadataDone),
];

/// Crash points of an undo.
const UNDO: [(&str, State); 7] = [
    ("planned", State::Planned),
    ("backup_copy", State::Planned),
    ("temp_written", State::TempWritten),
    ("verified", State::Verified),
    ("rename", State::Verified),
    ("renamed", State::Renamed),
    ("metadata_done", State::MetadataDone),
];

/// Whether a crash at `point` comes after the rename.
fn renamed(point: &str) -> bool {
    matches!(point, "rename" | "renamed" | "metadata_done")
}

/// Runs `txn_apply <args>`, which must abort at `point`.
fn crash_at(args: &[&Path], mode: &str, point: &str) {
    let status = Command::new(env!("CARGO_BIN_EXE_txn_apply"))
        .arg(mode)
        .args(args)
        .env(CRASH_ENV, point)
        .env_remove(BACKUP_ROOT_ENV)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("txn_apply runs");
    assert_eq!(
        status.signal(),
        Some(SIGABRT),
        "{mode} at {point}: {status}"
    );
}

/// Backups (files other than the journal and lock files) under the backup root.
fn backups_in(root: &Path) -> Vec<PathBuf> {
    files_under(root)
        .into_iter()
        .filter(|p| !p.ends_with(txn::JOURNAL_FILE) && p.extension().is_none_or(|e| e != "lock"))
        .collect()
}

/// Recovers once (expecting one transaction that reached `reached`), checks a second recovery
/// finds nothing and no temp or lock file is left; returns the outcome.
fn recover_one(lib: &Library, reached: State, what: &str) -> Outcome {
    let report = txn::recover(&lib.backups).expect("recovery");
    assert!(report.pending.is_empty(), "{what}: {report:?}");
    assert_eq!(report.recovered.len(), 1, "{what}: {report:?}");
    let r = &report.recovered[0];
    assert_eq!(r.reached, reached, "{what}");
    let again = txn::recover(&lib.backups).expect("again");
    assert!(
        again.recovered.is_empty() && again.pending.is_empty(),
        "{what}: idempotent"
    );
    assert_no_temps(lib.dir.path());
    let locks = files_under(&lib.backups.join("locks"));
    assert!(locks.is_empty(), "{what}: lock files left: {locks:?}");
    let last = journal_entries(&lib.backups).expect("journal");
    assert!(last.iter().all(|e| e.state.is_final()), "{what}");
    r.outcome
}

fn sidecars(dir: &Path) -> usize {
    files_under(dir)
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(SIDECAR_SUFFIX))
        .count()
}

fn in_place_case(name: &str, bytes: &[u8], point: &str, reached: State) {
    let what = format!("{name} in place at {point}");
    let lib = Library::new();
    let path = lib.add(name, bytes);
    let original = blake3_of(&path);
    crash_at(&[&path, &lib.backups], "apply", point);
    let outcome = recover_one(&lib, reached, &what);
    let entry = &journal_entries(&lib.backups).expect("journal")[0];
    let backups = backups_in(&lib.backups);
    if renamed(point) {
        assert_eq!(outcome, Outcome::Completed, "{what}");
        assert_eq!(Some(hex(&blake3_of(&path))), entry.output_blake3, "{what}");
        let audio = sc_io::read_all(&path).expect("the output decodes");
        assert_eq!(audio.frames(), 3000);
        assert_eq!(backups.len(), 1, "{what}: {backups:?}");
        assert_eq!(blake3_of(&backups[0]), original, "{what}: the backup");
        assert!(sidecar_path(&path).exists(), "{what}");
    } else {
        assert_eq!(outcome, Outcome::RolledBack, "{what}");
        assert_eq!(blake3_of(&path), original, "{what}: the original");
        assert!(backups.is_empty(), "{what}: {backups:?}");
        assert!(!sidecar_path(&path).exists(), "{what}");
    }
    match txn::undo(&path, &lib.backups) {
        Ok(u) => assert!(renamed(point) && u.restored_blake3 == original, "{what}"),
        Err(Error::NothingToUndo { .. }) => assert!(!renamed(point), "{what}"),
        Err(e) => panic!("{what}: undo: {e}"),
    }
    assert_eq!(blake3_of(&path), original, "{what}: after undo");
    assert_no_temps(lib.dir.path());
    assert_eq!(sidecars(&lib.music), 0, "{what}");
}

#[test]
fn every_crash_point_of_an_in_place_change_recovers() {
    let files = [("a.wav", wav(3000, 21)), ("b.flac", flac(3000, 22))];
    for (point, reached) in IN_PLACE {
        for (name, bytes) in &files {
            in_place_case(name, bytes, point, reached);
        }
    }
}

#[test]
fn every_crash_point_of_a_copy_to_a_folder_recovers() {
    for (point, reached) in TO_FOLDER {
        let what = format!("copy at {point}");
        let lib = Library::new();
        let path = lib.add("a.wav", &wav(3000, 23));
        let original = blake3_of(&path);
        let out = lib.dir.path().join("out");
        crash_at(&[&path, &lib.backups, &out], "folder", point);
        let outcome = recover_one(&lib, reached, &what);
        assert_eq!(blake3_of(&path), original, "{what}: the source");
        let written = out.join("a.wav");
        if renamed(point) {
            assert_eq!(outcome, Outcome::Completed, "{what}");
            let entry = &journal_entries(&lib.backups).expect("journal")[0];
            assert_eq!(
                Some(hex(&blake3_of(&written))),
                entry.output_blake3,
                "{what}"
            );
            assert!(sidecar_path(&written).exists(), "{what}");
        } else {
            assert_eq!(outcome, Outcome::RolledBack, "{what}");
            assert!(!written.exists(), "{what}: nothing at the destination");
            assert_eq!(sidecars(&out), 0, "{what}");
        }
        assert!(backups_in(&lib.backups).is_empty(), "{what}: no backup");
    }
}

#[test]
fn every_crash_point_of_an_undo_recovers() {
    for (point, reached) in UNDO {
        let what = format!("undo at {point}");
        let lib = Library::new();
        let path = lib.add("a.wav", &wav(3000, 24));
        let original = blake3_of(&path);
        let req = RenderRequest {
            gain_db: -3.0,
            ..RenderRequest::default()
        };
        let applied = txn::apply_in_place(
            &path,
            &req,
            &TxnOptions::new(&lib.backups),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("applied");
        crash_at(&[&path, &lib.backups], "undo", point);
        let outcome = recover_one(&lib, reached, &what);
        if renamed(point) {
            assert_eq!(outcome, Outcome::Completed, "{what}");
            assert_eq!(blake3_of(&path), original, "{what}: the original");
            assert!(!sidecar_path(&path).exists(), "{what}");
            assert!(
                matches!(
                    txn::undo(&path, &lib.backups),
                    Err(Error::NothingToUndo { .. })
                ),
                "{what}"
            );
        } else {
            assert_eq!(outcome, Outcome::RolledBack, "{what}");
            assert_eq!(
                blake3_of(&path),
                applied.output_blake3,
                "{what}: the output"
            );
            assert!(sidecar_path(&path).exists(), "{what}");
            txn::undo(&path, &lib.backups).expect("undo again");
            assert_eq!(blake3_of(&path), original, "{what}: after undo");
        }
        assert_eq!(
            backups_in(&lib.backups).len(),
            1,
            "{what}: the backup is kept"
        );
        assert_no_temps(lib.dir.path());
    }
}
