//! The crash matrix (feature `crash-test`): for every journaled step of an in-place change and
//! for a WAV and a FLAC file, the `txn_apply` program (`examples/txn_apply.rs`) is run with
//! `SC_TEST_CRASH_AFTER_STEP` naming the step, so it aborts right after journaling it. Recovery
//! must then leave the original (same BLAKE3) or the verified output (the journaled BLAKE3, and
//! it decodes) at the path, never anything else; no temp file and no lock file; the journal
//! entry recovered; the backup present (and equal to the original) whenever the rename
//! happened, and gone when it did not. A second recovery finds nothing, and undo leaves the
//! original.

mod common;

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{Library, assert_no_temps, blake3_of, files_under, flac, wav};
use sc_core::Error;
use sc_io::txn::{
    self, BACKUP_ROOT_ENV, CRASH_ENV, Outcome, SIDECAR_SUFFIX, State, hex, journal_entries,
    sidecar_path,
};

/// SIGABRT, what `std::process::abort` raises.
const SIGABRT: i32 = 6;

const STEPS: [State; 6] = [
    State::Planned,
    State::TempWritten,
    State::Verified,
    State::BackedUp,
    State::Renamed,
    State::MetadataDone,
];

/// The `txn_apply` program, built by Cargo for this test (a bin target of `sc-io` that needs
/// the same `crash-test` feature).
fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_txn_apply"))
}

/// Runs `txn_apply` on `path`, crashing after `step`.
fn crash_after(example: &Path, path: &Path, backups: &Path, step: State) {
    let status = Command::new(example)
        .arg(path)
        .arg(backups)
        .env(CRASH_ENV, step.as_str())
        .env_remove(BACKUP_ROOT_ENV)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("txn_apply runs");
    assert_eq!(
        status.signal(),
        Some(SIGABRT),
        "crash after {step:?}: {status}"
    );
}

/// Backups (files other than the journal) under the backup root.
fn backups_in(root: &Path) -> Vec<PathBuf> {
    files_under(root)
        .into_iter()
        .filter(|p| !p.ends_with(txn::JOURNAL_FILE) && p.extension().is_none_or(|e| e != "lock"))
        .collect()
}

fn crash_case(example: &Path, name: &str, bytes: &[u8], step: State) {
    let lib = Library::new();
    let path = lib.add(name, bytes);
    let original = blake3_of(&path);
    crash_after(example, &path, &lib.backups, step);

    let recovered = txn::recover(&lib.backups).expect("recovery").recovered;
    assert_eq!(recovered.len(), 1, "{name} after {step:?}");
    let r = &recovered[0];
    assert_eq!(r.reached, step);
    assert_eq!(r.path, path);
    let renamed = step >= State::Renamed;
    let want = if renamed {
        Outcome::Completed
    } else {
        Outcome::RolledBack
    };
    assert_eq!(r.outcome, want, "{name} after {step:?}");
    let entries = journal_entries(&lib.backups).expect("journal");
    let entry = &entries[0];
    assert_eq!(entry.state, State::Recovered);

    let now = blake3_of(&path);
    let backups = backups_in(&lib.backups);
    if renamed {
        assert_eq!(Some(hex(&now)), entry.output_blake3, "the verified output");
        let audio = sc_io::read_all(&path).expect("the output decodes");
        assert_eq!(audio.frames(), 3000);
        assert_eq!(backups.len(), 1, "one backup: {backups:?}");
        assert_eq!(
            blake3_of(&backups[0]),
            original,
            "the backup is the original"
        );
        assert!(sidecar_path(&path).exists());
    } else {
        assert_eq!(now, original, "the original");
        assert!(backups.is_empty(), "no backup left: {backups:?}");
        assert!(!sidecar_path(&path).exists());
    }
    assert_no_temps(lib.dir.path());
    let locks = files_under(&lib.backups.join("locks"));
    assert!(locks.is_empty(), "lock files left: {locks:?}");
    assert!(
        txn::recover(&lib.backups)
            .expect("again")
            .recovered
            .is_empty(),
        "recovery is idempotent"
    );

    match txn::undo(&path, &lib.backups) {
        Ok(u) => {
            assert!(renamed);
            assert_eq!(u.restored_blake3, original);
        }
        Err(Error::NothingToUndo { .. }) => assert!(!renamed),
        Err(e) => panic!("undo after {step:?}: {e}"),
    }
    assert_eq!(blake3_of(&path), original, "undo leaves the original");
    assert_no_temps(lib.dir.path());
    let sidecars = files_under(&lib.music)
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(SIDECAR_SUFFIX))
        .count();
    assert_eq!(sidecars, 0);
}

#[test]
fn every_crash_point_recovers_to_the_original_or_the_verified_output() {
    let example = example();
    let files = [("a.wav", wav(3000, 21)), ("b.flac", flac(3000, 22))];
    for step in STEPS {
        for (name, bytes) in &files {
            crash_case(&example, name, bytes, step);
        }
    }
}
