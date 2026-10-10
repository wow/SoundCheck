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

fn gain(gain_db: f64) -> ApplyRequest {
    ApplyRequest {
        gain_db,
        ..ApplyRequest::default()
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
    let none = recover_at_start(&s.base.join("no backups yet"), None);
    assert_eq!(
        none,
        RecoveryStatus::Finished {
            recovered: vec![],
            pending: vec![]
        }
    );
    // A journal that cannot be read (a folder in its place) is reported, not fatal.
    std::fs::create_dir_all(s.backups.join(sc_io::txn::JOURNAL_FILE)).expect("folder");
    let unreadable = recover_at_start(&s.backups, None);
    assert!(
        matches!(unreadable, RecoveryStatus::Failed { .. }),
        "{unreadable:?}"
    );
}

#[test]
fn undo_puts_the_original_back() {
    let s = scene();
    let before = std::fs::read(&s.file).expect("read");
    let opts = ApplyOptions::in_place(&s.backups);
    apply_file(&s.file, &gain(-2.0), &opts, &CancelToken::new()).expect("applied");
    let r = undo_file(&s.file, &s.backups, None).expect("undone");
    assert_eq!(r.path, s.file);
    assert_eq!(std::fs::read(&s.file).expect("restored"), before);
    assert!(matches!(
        undo_file(&s.file, &s.backups, None),
        Err(Error::NothingToUndo { .. })
    ));
}

#[test]
fn neutral_tags_reach_the_id3_chunk_and_bad_ones_change_nothing() {
    let s = scene();
    let before = std::fs::read(&s.file).expect("read");
    let opts = ApplyOptions::in_place(&s.backups);
    let bad = ApplyRequest {
        tags: vec![Tag::parse("TBPM=128").expect("parsed")],
        ..gain(-1.0)
    };
    let err = apply_file(&s.file, &bad, &opts, &CancelToken::new()).expect_err("refused");
    assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
    assert_eq!(std::fs::read(&s.file).expect("untouched"), before);
    // The hound WAV has no ID3 chunk: the edits are mapped, then reported as not added.
    let good = ApplyRequest {
        tags: vec![Tag::parse("BPM=128.00").expect("parsed")],
        ..gain(-1.0)
    };
    let r = apply_file(&s.file, &good, &opts, &CancelToken::new()).expect("applied");
    assert!(!r.render.tags_added && r.render.tags_not_added.is_some());
}

/// Puts a placeholder cache entry for `file`; returns its path.
fn cached(cache: &Cache, file: &Path) -> PathBuf {
    let entry = cache.entry_path(&sc_io::cache::nfc(file));
    std::fs::create_dir_all(cache.dir()).expect("cache dir");
    std::fs::write(&entry, b"{}").expect("entry");
    entry
}

#[test]
fn apply_removes_the_cache_entry_even_when_it_fails_after_the_render() {
    let s = scene();
    let cache = Cache::open(s.base.join("cache"));
    let entry = cached(&cache, &s.file);
    let before = std::fs::read(&s.file).expect("read");
    // A plan made from other bytes: refused after the render, before the rename.
    let req = ApplyRequest {
        source_blake3: Some([0; 32]),
        ..gain(-1.0)
    };
    let opts = ApplyOptions {
        cache: Some(cache.clone()),
        ..ApplyOptions::in_place(&s.backups)
    };
    let err = apply_file(&s.file, &req, &opts, &CancelToken::new()).expect_err("refused");
    assert!(matches!(err, Error::FileChanged { .. }), "{err}");
    assert_eq!(std::fs::read(&s.file).expect("read"), before);
    assert!(!entry.exists(), "the entry is removed whatever the outcome");

    // A change that succeeds, and its undo, remove it too.
    let entry = cached(&cache, &s.file);
    apply_file(&s.file, &gain(-1.0), &opts, &CancelToken::new()).expect("applied");
    assert!(!entry.exists());
    let entry = cached(&cache, &s.file);
    undo_file(&s.file, &s.backups, Some(&cache)).expect("undone");
    assert!(!entry.exists());

    // To a folder, the copy's entry goes and the source's stays.
    let out = s.base.join("out");
    let source_entry = cached(&cache, &s.file);
    let copy_entry = cached(&cache, &out.join("Track.wav"));
    let to_folder = ApplyOptions {
        place: Place::Folder(out),
        ..opts
    };
    apply_file(&s.file, &gain(-1.0), &to_folder, &CancelToken::new()).expect("copied");
    assert!(source_entry.exists());
    assert!(!copy_entry.exists());
}

#[test]
fn recovery_removes_the_entries_of_the_files_it_ended() {
    let s = scene();
    let cache = Cache::open(s.base.join("cache"));
    let entry = cached(&cache, &s.file);
    let other = cached(&cache, &s.base.join("music/Other.wav"));
    let report = RecoveryReport {
        recovered: vec![sc_io::txn::Recovered {
            txn: "t-1".into(),
            kind: sc_io::txn::TxnKind::InPlace,
            path: s.file.clone(),
            reached: sc_io::txn::State::Renamed,
            outcome: Outcome::Completed,
            notes: Vec::new(),
        }],
        pending: Vec::new(),
    };
    forget_recovered(Some(&cache), &report);
    assert!(!entry.exists());
    assert!(other.exists(), "files recovery did not touch keep theirs");
}
