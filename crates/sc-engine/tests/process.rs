//! `run_batch` with `Task::Process`, without the model: the event order per file, a cancel that
//! leaves no temp file and every journal entry finished or rolled back, Library keeping the frame
//! count, the analysis cache and undo, the source hash check, and the wait for the start-up
//! recovery. Fixtures are synthetic; the grid cases are in `process_grid.rs`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::export::*;
use sc_core::Error;
use sc_core::export::{BatchMode, Cut, ExportOutcome, ExportSkip, Place};
use sc_engine::{
    ApplyOptions, ApplyRequest, CancelToken, EngineEvent, RecoveryGate, apply_file, run_batch,
};
use sc_io::cache::Cache;
use sc_io::txn::{self, State, sidecar};

#[test]
fn events_in_order() {
    let lib = Lib::new();
    let music = lib.music();
    let written = common::tone_wav(&music, "a.wav", 3.0, 0.1);
    let skipped = tone_at(&music, "hi-res.wav", 96_000, 2.0);
    let broken = common::corrupt_wav(&music, "broken.wav");
    let files = batch(&[written.clone(), skipped, broken]);
    let s = settings(
        &lib,
        common::loudness_only(),
        process(&lib, BatchMode::Library, None),
    );
    let events = run(&files, &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    assert_eq!(names(&events, 2), ["started", "exportSkipped"]);
    assert_eq!(names(&events, 3), ["started", "failed"]);
    assert!(events.iter().any(|e| matches!(
        e,
        EngineEvent::ExportSkipped { outcome, .. } if **outcome == ExportOutcome::Skip {
            reason: ExportSkip::NotDjSafeRate { sample_rate_hz: 96_000 }
        }
    )));
    let done = events
        .iter()
        .find_map(|e| match e {
            EngineEvent::Done { done, .. } => Some(done),
            _ => None,
        })
        .expect("done");
    assert_eq!(done.output, written);
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    let analysis = done.analysis.as_ref().expect("the output analysed");
    assert_eq!(analysis.path, sc_io::cache::nfc(&written));
    // The output's analysis is now what the cache serves for the file.
    let (nfc, key) = Cache::key_for(&written, &common::loudness_only()).expect("key");
    assert_eq!(lib.cache().get(&nfc, &key).as_ref(), Some(&**analysis));
    // Its gain landed on the target: S-P95 at -11 LUFS.
    let p95 = analysis.loudness.short_term_p95.expect("S-P95").0;
    assert!((p95 + 11.0).abs() < 0.05, "{p95}");
}

#[test]
fn library_keeps_frame_count() {
    let lib = Lib::new();
    let music = lib.music();
    let path = common::tone_wav(&music, "a.wav", 3.0, 0.1);
    let before = frames(&path);
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .expect("mtime");
    let s = settings(
        &lib,
        common::loudness_only(),
        process(&lib, BatchMode::Library, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    assert_eq!(frames(&path), before);
    assert_eq!(
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .expect("mtime"),
        mtime,
        "Library keeps the modification time"
    );
    let doc = sidecar_of(&path);
    assert_eq!(doc.schema, sidecar::SIDECAR_SCHEMA);
    let export = doc.export.expect("the export is recorded");
    assert_eq!(export.plan.cut, Cut::Library);
    assert_eq!(export.plan.expect_frames, u64::from(before));
    assert_eq!(doc.render.frames_out, doc.render.frames_in);
    assert_eq!(export.source.frames, u64::from(before));
    assert!(export.source.short_term_p95.is_some());

    // An output of another length than planned never replaces anything.
    let other = common::tone_wav(&music, "c.wav", 3.0, 0.1);
    let before_other = hash(&other);
    let mut wrong = export.clone();
    wrong.plan.expect_frames += 1;
    let request = ApplyRequest {
        gain_db: -1.0,
        export: Some(wrong),
        ..ApplyRequest::default()
    };
    let err = apply_file(
        &other,
        &request,
        &ApplyOptions::in_place(lib.backups()),
        &CancelToken::new(),
    )
    .expect_err("refused");
    assert!(matches!(err, Error::VerifyFailed { .. }), "{err}");
    assert_eq!(hash(&other), before_other);
    assert_eq!(temps(lib.dir.path()), Vec::<PathBuf>::new());

    // To a folder: the copy has the same length and the original is untouched.
    let lib2 = Lib::new();
    let source = common::tone_wav(&lib2.music(), "b.wav", 3.0, 0.1);
    let original = hash(&source);
    let out = lib2.dir.path().join("out");
    let s = settings(
        &lib2,
        common::loudness_only(),
        process(&lib2, BatchMode::Library, Some(out.clone())),
    );
    let events = run(
        &batch(std::slice::from_ref(&source)),
        &s,
        &CancelToken::new(),
    );
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    assert_eq!(hash(&source), original);
    assert_eq!(frames(&out.join("b.wav")), frames(&source));
}

#[test]
fn cancel_leaves_no_temp() {
    let lib = Lib::new();
    let music = lib.music();
    let paths: Vec<PathBuf> = (0..6)
        .map(|i| common::tone_wav(&music, &format!("t{i}.wav"), 4.0, 0.1))
        .collect();
    let originals: Vec<[u8; 32]> = paths.iter().map(|p| hash(p)).collect();
    let files = batch(&paths);
    let s = settings(
        &lib,
        common::loudness_only(),
        process(&lib, BatchMode::Library, None),
    );
    let cancel = CancelToken::new();
    let mut events = Vec::new();
    run_batch(&files, &s, &cancel, &mut |e| {
        if matches!(e, EngineEvent::Processing { .. }) {
            cancel.cancel();
        }
        events.push(e);
    })
    .expect("batch starts");
    let mut cancelled = 0;
    for (file, original) in files.iter().zip(&originals) {
        let n = names(&events, file.file_id);
        match n.last().copied() {
            Some("done") => assert_ne!(hash(&file.path), *original),
            Some("cancelled") => {
                cancelled += 1;
                assert_eq!(hash(&file.path), *original, "{}", file.path.display());
            }
            other => panic!("{other:?} for {}", file.path.display()),
        }
    }
    assert!(cancelled > 0);
    assert_eq!(temps(lib.dir.path()), Vec::<PathBuf>::new());
    for entry in txn::journal_entries(&lib.backups()).expect("journal") {
        assert!(
            matches!(entry.state, State::Done | State::Failed),
            "{} ended {:?}",
            entry.txn,
            entry.state
        );
    }
}

#[test]
fn recovery_before_first_write() {
    let lib = Lib::new();
    let path = common::tone_wav(&lib.music(), "a.wav", 3.0, 0.1);
    let original = hash(&path);
    let gate = Arc::new(RecoveryGate::running());
    let mut p = process(&lib, BatchMode::Library, None);
    p.recovery = Arc::clone(&gate);
    let s = settings(&lib, common::loudness_only(), p);
    let files = batch(std::slice::from_ref(&path));
    let (tx, rx) = std::sync::mpsc::channel();
    let job = std::thread::spawn(move || {
        run_batch(&files, &s, &CancelToken::new(), &mut |e| {
            let _ = tx.send(file_of(&e).is_some());
        })
    });
    // While recovery runs, nothing happens to any file.
    std::thread::sleep(Duration::from_millis(300));
    assert!(rx.try_iter().all(|per_file| !per_file));
    assert_eq!(hash(&path), original);
    assert!(!lib.backups().exists(), "no journal before recovery ends");
    gate.set(finished());
    let summary = job.join().expect("job").expect("batch");
    assert_eq!(summary.written, 1);
    assert_ne!(hash(&path), original);

    // A cancel while waiting cancels every file and touches none.
    let lib = Lib::new();
    let path = common::tone_wav(&lib.music(), "b.wav", 3.0, 0.1);
    let original = hash(&path);
    let mut p = process(&lib, BatchMode::Library, None);
    p.recovery = Arc::new(RecoveryGate::running());
    let s = settings(&lib, common::loudness_only(), p);
    let cancel = CancelToken::new();
    cancel.cancel();
    let events = run(&batch(std::slice::from_ref(&path)), &s, &cancel);
    assert_eq!(names(&events, 1), ["cancelled"]);
    assert_eq!(hash(&path), original);
}

#[test]
fn invalid_settings_stop_the_batch_before_any_file() {
    let lib = Lib::new();
    let path = common::tone_wav(&lib.music(), "a.wav", 3.0, 0.1);
    let mut p = process(&lib, BatchMode::Library, None);
    p.export.place = Place::Folder;
    let s = settings(&lib, common::loudness_only(), p);
    let err =
        run_batch(&batch(&[path]), &s, &CancelToken::new(), &mut |_| {}).expect_err("refused");
    assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
}

/// S-P95 of `path` measured afresh, without the cache.
fn measured_p95(path: &std::path::Path) -> f64 {
    sc_engine::Analyzer::load(common::loudness_only(), None, CancelToken::new())
        .expect("analyzer")
        .analyze(path)
        .expect("analysed")
        .record
        .loudness
        .short_term_p95
        .expect("S-P95")
        .0
}

#[test]
fn a_cancel_at_written_leaves_no_stale_cache_entry() {
    // A Library export keeps the modification time and (without a tag) the length, so only the
    // file's identity tells the written file from the original in the cache.
    let lib = Lib::new();
    let path = common::tone_wav(&lib.music(), "a.wav", 3.0, 0.1);
    let s = settings(
        &lib,
        common::loudness_only(),
        process(&lib, BatchMode::Library, None),
    );
    let cancel = CancelToken::new();
    run_batch(&batch(std::slice::from_ref(&path)), &s, &cancel, &mut |e| {
        if matches!(e, EngineEvent::Written { .. }) {
            cancel.cancel();
        }
    })
    .expect("batch starts");
    assert!((measured_p95(&path) + 11.0).abs() < 0.05);
    let (nfc, key) = Cache::key_for(&path, &common::loudness_only()).expect("key");
    assert!(
        lib.cache().get(&nfc, &key).is_none(),
        "the original's analysis must not serve the written file"
    );
    // A second export finds the file at the target and does not apply the gain again.
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(names(&events, 1).last(), Some(&"done"));
    let p95 = measured_p95(&path);
    assert!((p95 + 11.0).abs() < 0.05, "{p95}");
}

#[test]
fn after_undo_the_cache_describes_the_original() {
    let lib = Lib::new();
    let path = common::tone_wav(&lib.music(), "a.wav", 3.0, 0.1);
    let original = measured_p95(&path);
    let s = settings(
        &lib,
        common::loudness_only(),
        process(&lib, BatchMode::Library, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(names(&events, 1).last(), Some(&"done"));
    sc_engine::undo_file(&path, &lib.backups()).expect("undone");
    let cached = sc_engine::Analyzer::load(
        common::loudness_only(),
        Some(lib.cache()),
        CancelToken::new(),
    )
    .expect("analyzer")
    .analyze(&path)
    .expect("analysed")
    .record
    .loudness
    .short_term_p95
    .expect("S-P95")
    .0;
    assert!((cached - original).abs() < 1e-9, "{cached} vs {original}");
}
