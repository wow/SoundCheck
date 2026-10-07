//! Unit tests of `apply.rs` with injected faults: a file edited during processing (same length,
//! modification time put back), a rename that fails after the output landed, a carried chunk
//! damaged in the temp file; and the owner and backup-root rules.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest};

use crate::txn::tests::wav;
use crate::txn::{Hooks, State, Transaction, TxnOptions, journal_entries, recover, sidecar_path};

struct Scene {
    _dir: tempfile::TempDir,
    music: PathBuf,
    backups: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
}

/// `music/a.wav` (a WAV with a `LIST` chunk after the audio) and an empty backup root.
fn scene() -> Scene {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let music = base.join("music");
    std::fs::create_dir(&music).expect("music");
    let mut bytes = wav(3000);
    bytes.extend_from_slice(b"LIST\x08\x00\x00\x00INFOabcd");
    let riff = u32::try_from(bytes.len() - 8).expect("small");
    bytes[4..8].copy_from_slice(&riff.to_le_bytes());
    let path = music.join("a.wav");
    std::fs::write(&path, &bytes).expect("fixture");
    Scene {
        _dir: dir,
        music,
        backups: base.join("backups"),
        path,
        bytes,
    }
}

fn run(s: &Scene, hooks: Hooks) -> sc_core::Result<crate::txn::TxnReport> {
    let tx = Transaction {
        hooks,
        ..Transaction::default()
    };
    let req = RenderRequest {
        gain_db: -2.0,
        ..RenderRequest::default()
    };
    tx.apply_in_place(
        &s.path,
        &req,
        &TxnOptions::new(&s.backups),
        &AtomicBool::new(false),
    )
}

/// Names in `dir` other than the fixture.
fn others(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n != "a.wav")
        .collect();
    names.sort();
    names
}

/// Backups (any file below a dated folder) in `root`.
fn backups(root: &Path) -> usize {
    fn count(d: &Path) -> usize {
        std::fs::read_dir(d).map_or(0, |l| {
            l.flatten()
                .map(|e| {
                    let p = e.path();
                    if p.is_dir() { count(&p) } else { 1 }
                })
                .sum()
        })
    }
    std::fs::read_dir(root).map_or(0, |l| {
        l.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("20"))
            .map(|e| count(&e.path()))
            .sum()
    })
}

/// Rewrites one audio byte of `source` in place and puts its modification time back: length
/// and mtime are as before, the change time is not.
fn edit_keeping_mtime(_temp: &Path, source: &Path) {
    let mtime = std::fs::metadata(source)
        .and_then(|m| m.modified())
        .expect("mtime");
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(source)
        .expect("source");
    f.seek(SeekFrom::Start(100)).expect("seek");
    let mut b = [0_u8; 1];
    std::io::Read::read_exact(&mut f, &mut b).expect("read");
    f.seek(SeekFrom::Start(100)).expect("seek");
    f.write_all(&[b[0] ^ 1]).expect("write");
    f.set_times(std::fs::FileTimes::new().set_modified(mtime))
        .expect("mtime back");
}

#[test]
fn an_edit_during_processing_that_hides_its_mtime_is_noticed() {
    let s = scene();
    let hooks = Hooks {
        after_temp_written: Some(edit_keeping_mtime),
        ..Hooks::default()
    };
    match run(&s, hooks) {
        Err(Error::FileChanged { path, .. }) => assert_eq!(path, s.path),
        other => panic!("{other:?}"),
    }
    let now = std::fs::read(&s.path).expect("file");
    assert_eq!(now.len(), s.bytes.len());
    assert_ne!(now, s.bytes, "the edit is kept, not overwritten");
    assert_eq!(others(&s.music), Vec::<String>::new(), "no temp left");
    assert_eq!(backups(&s.backups), 0, "no backup left");
    let e = &journal_entries(&s.backups).expect("journal")[0];
    assert_eq!(e.state, State::Failed);
}

/// Puts the temp file's bytes at the target and removes the temp, so the rename then fails
/// although the output is in place.
fn land_then_fail(temp: &Path, target: &Path) {
    std::fs::copy(temp, target).expect("output in place");
    std::fs::remove_file(temp).expect("temp gone");
}

#[test]
fn a_rename_error_after_the_output_landed_keeps_the_backup_for_recovery() {
    let s = scene();
    let hooks = Hooks {
        before_rename: Some(land_then_fail),
        ..Hooks::default()
    };
    assert!(matches!(run(&s, hooks), Err(Error::Io { .. })));
    assert_eq!(backups(&s.backups), 1, "the backup is kept");
    let e = &journal_entries(&s.backups).expect("journal")[0];
    assert!(!e.state.is_final(), "left pending: {:?}", e.state);
    let r = recover(&s.backups).expect("recovery");
    assert_eq!(r.recovered.len(), 1);
    assert_eq!(r.recovered[0].outcome, crate::txn::Outcome::Completed);
    assert!(sidecar_path(&s.path).exists());
    crate::txn::undo(&s.path, &s.backups).expect("undo");
    assert_eq!(std::fs::read(&s.path).expect("file"), s.bytes);
}

/// Flips a byte inside the temp file's `LIST` chunk (carried, so it must match the source).
fn damage_list(temp: &Path, _source: &Path) {
    let mut bytes = std::fs::read(temp).expect("temp");
    let at = bytes
        .windows(4)
        .rposition(|w| w == b"abcd")
        .expect("LIST payload");
    bytes[at] ^= 0x20;
    std::fs::write(temp, bytes).expect("damaged");
}

#[test]
fn a_damaged_carried_chunk_fails_verification() {
    let s = scene();
    let hooks = Hooks {
        after_temp_written: Some(damage_list),
        ..Hooks::default()
    };
    match run(&s, hooks) {
        Err(Error::VerifyFailed { detail, .. }) => assert!(detail.contains("LIST"), "{detail}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&s.path).expect("file"), s.bytes);
    assert_eq!(others(&s.music), Vec::<String>::new());
    assert_eq!(backups(&s.backups), 0);
}

#[test]
fn two_transactions_on_one_file_run_one_after_the_other() {
    let s = scene();
    let results: Vec<_> = std::thread::scope(|scope| {
        let a = scope.spawn(|| run(&s, Hooks::default()));
        let b = scope.spawn(|| run(&s, Hooks::default()));
        vec![a.join().expect("a"), b.join().expect("b")]
    });
    let done = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        results
            .iter()
            .all(|r| r.is_ok() || matches!(r, Err(Error::FileChanged { .. }))),
        "{results:?}"
    );
    assert_eq!(backups(&s.backups), done);
    for _ in 0..done {
        crate::txn::undo(&s.path, &s.backups).expect("undo");
    }
    assert_eq!(
        std::fs::read(&s.path).expect("file"),
        s.bytes,
        "back to the original"
    );
}

#[test]
fn other_users_files_are_refused_unless_running_as_root() {
    use crate::txn::meta::owned_by_other;
    assert!(owned_by_other(501, 502));
    assert!(!owned_by_other(501, 501));
    assert!(
        !owned_by_other(501, 0),
        "the superuser may give the file back"
    );
}

#[test]
fn paths_in_the_backup_root_are_recognised_even_before_it_exists() {
    use crate::txn::is_under_backup_root;
    let s = scene();
    assert!(!is_under_backup_root(&s.path, &s.backups).expect("checked"));
    let inside = s.backups.join("2026-10-07/Disk/a.wav");
    assert!(is_under_backup_root(&inside, &s.backups).expect("checked"));
    std::fs::create_dir_all(inside.parent().expect("parent")).expect("folders");
    std::fs::write(&inside, b"x").expect("file");
    assert!(is_under_backup_root(&inside, &s.backups).expect("checked"));
    let refused = Transaction::default().apply_in_place(
        &inside,
        &RenderRequest::default(),
        &TxnOptions::new(&s.backups),
        &AtomicBool::new(false),
    );
    assert!(
        matches!(refused, Err(Error::InvalidArgument(_))),
        "{refused:?}"
    );
}
