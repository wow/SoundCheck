//! Unit tests of `txn.rs`: a file applied in place (backup, sidecar), copied into a folder
//! (original untouched), a cancelled apply, and recovery that never fails the start.

use std::path::{Path, PathBuf};

use sc_core::Error;

use super::*;

/// A 0.5 s 44.1 kHz stereo 16-bit WAV of a ramp at about -12 dBFS.
fn wav(dir: &Path) -> PathBuf {
    let path = dir.join("Track.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for i in 0..44_100_i32 {
        let s = i16::try_from((i % 2000) * 4 - 4000).expect("small");
        w.write_sample(s).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

fn gain(gain_db: f64) -> RenderRequest {
    RenderRequest {
        gain_db,
        ..RenderRequest::default()
    }
}

struct Scene {
    _dir: tempfile::TempDir,
    base: PathBuf,
    file: PathBuf,
    backups: PathBuf,
}

fn scene() -> Scene {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let music = base.join("music");
    std::fs::create_dir_all(&music).expect("music");
    let file = wav(&music);
    Scene {
        _dir: dir,
        backups: base.join("backups"),
        base,
        file,
    }
}

#[test]
fn in_place_backs_up_and_writes_a_sidecar() {
    let s = scene();
    let before = std::fs::read(&s.file).expect("read");
    let opts = ApplyOptions::in_place(&s.backups);
    let r = apply_file(&s.file, &gain(-3.2), &opts, &CancelToken::new()).expect("applied");
    assert_eq!(r.output, s.file);
    let backup = r.backup.expect("a backup");
    assert!(backup.starts_with(&s.backups), "{}", backup.display());
    assert_eq!(std::fs::read(&backup).expect("backup"), before);
    assert_ne!(std::fs::read(&s.file).expect("output"), before);
    assert!(r.sidecar.expect("a sidecar").is_file());
}

#[test]
fn a_folder_copy_leaves_the_original_and_needs_no_backup() {
    let s = scene();
    let before = std::fs::read(&s.file).expect("read");
    let out = s.base.join("out");
    let opts = ApplyOptions {
        place: Place::Folder(out.clone()),
        sidecar: false,
        ..ApplyOptions::in_place(&s.backups)
    };
    let r = apply_file(&s.file, &gain(-1.0), &opts, &CancelToken::new()).expect("applied");
    assert_eq!(r.output, out.join("Track.wav"));
    assert!(r.backup.is_none() && r.sidecar.is_none());
    assert_eq!(std::fs::read(&s.file).expect("original"), before);
}

#[test]
fn a_cancelled_apply_changes_nothing() {
    let s = scene();
    let before = std::fs::read(&s.file).expect("read");
    let cancel = CancelToken::new();
    cancel.cancel();
    let err = apply_file(
        &s.file,
        &gain(-3.0),
        &ApplyOptions::in_place(&s.backups),
        &cancel,
    )
    .expect_err("cancelled");
    assert!(matches!(err, Error::Cancelled), "{err}");
    assert_eq!(std::fs::read(&s.file).expect("original"), before);
}

#[test]
fn recovery_at_start_never_fails() {
    let s = scene();
    let none = recover_at_start(&s.base.join("no backups yet"));
    assert!(none.recovered.is_empty() && none.pending.is_empty());
    // A journal that cannot be read (a folder in its place) is logged, not fatal.
    std::fs::create_dir_all(s.backups.join(sc_io::txn::JOURNAL_FILE)).expect("folder");
    let unreadable = recover_at_start(&s.backups);
    assert!(unreadable.recovered.is_empty() && unreadable.pending.is_empty());
}

#[test]
fn undo_puts_the_original_back() {
    let s = scene();
    let before = std::fs::read(&s.file).expect("read");
    let opts = ApplyOptions::in_place(&s.backups);
    apply_file(&s.file, &gain(-2.0), &opts, &CancelToken::new()).expect("applied");
    let r = undo_file(&s.file, &s.backups).expect("undone");
    assert_eq!(r.path, s.file);
    assert_eq!(std::fs::read(&s.file).expect("restored"), before);
    assert!(matches!(
        undo_file(&s.file, &s.backups),
        Err(Error::NothingToUndo { .. })
    ));
}
