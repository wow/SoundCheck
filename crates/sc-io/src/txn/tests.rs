//! Unit tests of `txn/mod.rs`: a temp file that does not verify (WAV and FLAC, corrupted
//! through the test hook) leaves the original untouched, no temp and no backup, and journals
//! the failure; the default backup root. End-to-end behaviour is in `tests/txn.rs`, the crash
//! matrix in `tests/crash.rs`.

use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;

use super::*;
use crate::flac::test_build;

/// A 16-bit stereo 44.1 kHz WAV of `frames` frames.
fn wav(frames: usize) -> Vec<u8> {
    let data: Vec<u8> = test_build::samples(frames, 2, 16, 7)
        .iter()
        .flat_map(|s| i16::try_from(*s).expect("16-bit").to_le_bytes())
        .collect();
    let mut f = b"RIFF".to_vec();
    let len = |n: usize| u32::try_from(n).expect("small").to_le_bytes();
    f.extend_from_slice(&len(4 + 24 + 8 + data.len()));
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&len(16));
    f.extend_from_slice(&[1, 0, 2, 0]);
    f.extend_from_slice(&44_100_u32.to_le_bytes());
    f.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    f.extend_from_slice(&[4, 0, 16, 0]);
    f.extend_from_slice(b"data");
    f.extend_from_slice(&len(data.len()));
    f.extend_from_slice(&data);
    f
}

/// A 16-bit stereo 44.1 kHz FLAC of `frames` frames.
fn flac(frames: usize) -> Vec<u8> {
    let data = test_build::samples(frames, 2, 16, 9);
    let (frames, info) = test_build::encode(&data, 44_100, 2, 16);
    test_build::file(&[], &info, &[], &frames, &[])
}

/// Flips one byte 200 bytes before the end of the file (inside the audio of both fixtures).
fn corrupt(path: &Path, _source: &Path) {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("temp file");
    let at = f.seek(SeekFrom::End(-200)).expect("seek");
    let mut b = [0_u8; 1];
    std::io::Read::read_exact(&mut f, &mut b).expect("read");
    f.seek(SeekFrom::Start(at)).expect("seek");
    f.write_all(&[b[0] ^ 0x5A]).expect("write");
}

fn verify_failure_leaves_nothing(name: &str, bytes: &[u8]) {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let music = base.join("music");
    std::fs::create_dir(&music).expect("music");
    let path = music.join(name);
    std::fs::write(&path, bytes).expect("fixture");
    let backups = base.join("backups");
    let tx = Transaction {
        hooks: Hooks {
            after_temp_written: Some(corrupt),
            ..Hooks::default()
        },
        ..Transaction::default()
    };
    let req = RenderRequest {
        gain_db: -2.0,
        ..RenderRequest::default()
    };
    let opts = TxnOptions::new(&backups);
    match tx.apply_in_place(&path, &req, &opts, &AtomicBool::new(false)) {
        Err(Error::VerifyFailed { path: p, detail }) => {
            assert_eq!(p, path);
            assert!(!detail.is_empty(), "a reason");
        }
        other => panic!("{name}: {other:?}"),
    }
    assert_eq!(std::fs::read(&path).expect("original"), bytes);
    let left: Vec<_> = std::fs::read_dir(&music)
        .expect("list")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(
        left,
        vec![std::ffi::OsString::from(name)],
        "{name}: {left:?}"
    );
    let entries = journal_entries(&backups).expect("journal");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        (entries[0].state, entries[0].reached),
        (State::Failed, State::TempWritten)
    );
    let dated: Vec<_> = std::fs::read_dir(&backups)
        .expect("list")
        .map(|e| e.expect("entry").file_name())
        .filter(|n| n != "journal.jsonl" && n != "locks")
        .collect();
    assert!(dated.is_empty(), "nothing backed up: {dated:?}");
    assert!(
        recover(&backups).expect("recovery").recovered.is_empty(),
        "nothing to recover"
    );
}

#[test]
fn a_wav_temp_that_does_not_verify_leaves_the_original() {
    verify_failure_leaves_nothing("a.wav", &wav(5000));
}

#[test]
fn a_flac_temp_that_does_not_verify_leaves_the_original() {
    verify_failure_leaves_nothing("a.flac", &flac(5000));
}

#[test]
fn the_default_backup_root_is_in_the_music_folder() {
    let root = default_backup_root().expect("a home folder");
    match std::env::var_os(BACKUP_ROOT_ENV).filter(|v| !v.is_empty()) {
        Some(v) => assert_eq!(root, PathBuf::from(v)),
        None => assert!(root.ends_with(BACKUP_FOLDER_NAME), "{root:?}"),
    }
}
