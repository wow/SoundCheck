//! `run_batch` with `Task::Process`: the event order per file, a cancel that leaves no temp file
//! and every journal entry finished or rolled back, Library keeping the frame count, the wait for
//! the start-up recovery, and, with the model files, a Prepare cut that puts bar 1 at the lead
//! and a confirmed grid edit that survives the export. Fixtures are synthetic.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sc_core::analysis::{AnalysisSettings, GridEdit};
use sc_core::export::{BatchMode, Cut, ExportOutcome, ExportSettings, ExportSkip, Place};
use sc_core::ipc::RecoveryStatus;
use sc_core::plan::DecideSettings;
use sc_core::{AudioSpec, Error, testsig};
use sc_engine::{
    Analyzer, ApplyOptions, ApplyRequest, BatchFile, BatchSettings, CancelToken, EngineEvent,
    ProcessSettings, RecoveryGate, Task, apply_file, apply_saved, run_batch, save_edit,
};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;
use sc_io::txn::{self, State, TEMP_MARKER, sidecar};

/// Folders of one test: the music, backups, cache, edits.
struct Lib {
    dir: tempfile::TempDir,
}

impl Lib {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }
    fn music(&self) -> PathBuf {
        let p = self.dir.path().join("music");
        std::fs::create_dir_all(&p).expect("music folder");
        p
    }
    fn backups(&self) -> PathBuf {
        self.dir.path().join("backups")
    }
    fn cache(&self) -> Cache {
        Cache::open(self.dir.path().join("cache"))
    }
    fn edits(&self) -> EditStore {
        EditStore::open(self.dir.path().join("edits"))
    }
}

fn batch(paths: &[PathBuf]) -> Vec<BatchFile> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| BatchFile {
            file_id: u32::try_from(i).expect("few files") + 1,
            path: p.clone(),
            duration_hint: None,
        })
        .collect()
}

/// Process settings for `lib` in `mode`, in place unless `out` is given, with the gate open.
fn process(lib: &Lib, mode: BatchMode, out: Option<PathBuf>) -> ProcessSettings {
    ProcessSettings {
        decide: DecideSettings::dj(),
        export: ExportSettings {
            place: if out.is_some() {
                Place::Folder
            } else {
                Place::InPlace
            },
            ..ExportSettings::new(mode)
        },
        out_dir: out,
        backup_root: lib.backups(),
        edits: Some(lib.edits()),
        recovery: Arc::new(RecoveryGate::new(finished())),
    }
}

fn finished() -> RecoveryStatus {
    RecoveryStatus::Finished {
        recovered: Vec::new(),
        pending: Vec::new(),
    }
}

fn settings(lib: &Lib, analysis: AnalysisSettings, p: ProcessSettings) -> BatchSettings {
    BatchSettings {
        analysis,
        workers: 2,
        cache: Some(lib.cache()),
        task: Task::Process(Box::new(p)),
    }
}

fn run(files: &[BatchFile], s: &BatchSettings, cancel: &CancelToken) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    run_batch(files, s, cancel, &mut |e| events.push(e)).expect("batch starts");
    events
}

fn file_of(event: &EngineEvent) -> Option<u32> {
    match event {
        EngineEvent::Started { file_id }
        | EngineEvent::Progress { file_id, .. }
        | EngineEvent::Analysed { file_id, .. }
        | EngineEvent::Failed { file_id, .. }
        | EngineEvent::Cancelled { file_id }
        | EngineEvent::Processing { file_id, .. }
        | EngineEvent::Written { file_id, .. }
        | EngineEvent::ExportSkipped { file_id, .. }
        | EngineEvent::Done { file_id, .. } => Some(*file_id),
        EngineEvent::Batch(_) => None,
    }
}

/// The names of `file`'s events in order, progress left out.
fn names(events: &[EngineEvent], file: u32) -> Vec<&'static str> {
    events
        .iter()
        .filter(|e| file_of(e) == Some(file))
        .filter_map(|e| match e {
            EngineEvent::Started { .. } => Some("started"),
            EngineEvent::Progress { .. } => None,
            EngineEvent::Analysed { .. } => Some("analysed"),
            EngineEvent::Failed { .. } => Some("failed"),
            EngineEvent::Cancelled { .. } => Some("cancelled"),
            EngineEvent::Processing { .. } => Some("processing"),
            EngineEvent::Written { .. } => Some("written"),
            EngineEvent::ExportSkipped { .. } => Some("exportSkipped"),
            EngineEvent::Done { .. } => Some("done"),
            EngineEvent::Batch(_) => None,
        })
        .collect()
}

/// A 16-bit stereo WAV of a 1 kHz tone at `rate`.
fn tone_at(dir: &Path, name: &str, rate: u32, seconds: f64) -> PathBuf {
    let path = dir.join(name);
    let buf = testsig::sine(AudioSpec::new(rate, 2), 1000.0, 0.1, seconds);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

fn frames(path: &Path) -> u32 {
    hound::WavReader::open(path).expect("wav").duration()
}

fn hash(path: &Path) -> [u8; 32] {
    txn::hash_file(path).expect("hash").1
}

/// Every file under `dir` whose name marks a transaction's temp file.
fn temps(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().contains(TEMP_MARKER) {
                found.push(p);
            }
        }
    }
    found
}

fn sidecar_of(path: &Path) -> sidecar::SidecarDoc {
    sidecar::read(&txn::sidecar_path(path)).expect("sidecar reads")
}

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

/// A 120 BPM click track (bar 1 0.3 s in) analysed with the grid, and its cache.
fn click(lib: &Lib) -> PathBuf {
    common::click_wav(&lib.music(), 120.0, 64, 0.3)
}

/// The first sample index (frames) whose amplitude passes 1 % of full scale.
fn first_sound(path: &Path) -> u64 {
    let mut reader = hound::WavReader::open(path).expect("wav");
    let channels = u64::from(reader.spec().channels);
    let at = reader
        .samples::<i16>()
        .position(|s| s.expect("sample").unsigned_abs() > 327)
        .expect("sound");
    at as u64 / channels
}

#[test]
fn prepare_puts_bar_1_at_the_lead() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let frames_in = u64::from(frames(&path));
    let sound_in = first_sound(&path);
    let s = settings(
        &lib,
        common::with_grid(),
        process(&lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let doc = sidecar_of(&path);
    let export = doc.export.expect("export recorded");
    let trim = export.plan.trim_frames;
    assert!(matches!(export.plan.cut, Cut::Cut { frames, .. } if frames == trim));
    assert_eq!(doc.render.trim_frames(), trim);
    assert_eq!(u64::from(frames(&path)), frames_in - trim);
    // Bar 1 lands between the lead (5 ms: 220.5 samples) and 1 ms plus a sample after it.
    let grid = export.grid.expect("grid exported");
    let lead: f64 = 220.5;
    let bar1 = grid.first_bar_line.0 as f64;
    assert!(bar1 >= lead.floor() && bar1 <= lead + 44.1 + 1.0, "{bar1}");
    assert_eq!(grid.bar1, grid.first_bar_line, "bar 1 is the first line");
    assert_eq!(grid.bpm.0, 120.0);
    // The audio moved by exactly the cut.
    assert_eq!(first_sound(&path), sound_in - trim);
}

#[test]
fn confirmed_edit_survives_export() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let analysis = common::with_grid();
    let mut analyzer =
        Analyzer::load(analysis.clone(), Some(lib.cache()), CancelToken::new()).expect("model");
    let record = analyzer.analyze(&path).expect("analysed").record;
    let anchor = record.grid.as_ref().expect("grid").anchor;
    // The user confirms the analysed grid by ear (an empty edit, confirmed).
    let edit = GridEdit::default();
    save_edit(&lib.edits(), &record, analysis.bpm_range, &edit, true).expect("saved");
    let s = settings(
        &lib,
        analysis.clone(),
        process(&lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let done = events
        .iter()
        .find_map(|e| match e {
            EngineEvent::Done { done, .. } => Some(done),
            _ => None,
        })
        .expect("done");
    assert!(done.edit_carried && done.edit.confirmed, "{:?}", done.notes);
    let trim = sidecar_of(&path).export.expect("export").plan.trim_frames;
    assert!(trim > 0);
    let out_grid = done
        .analysis
        .as_ref()
        .and_then(|r| r.grid.clone())
        .expect("grid");
    let expected = anchor.0 - trim;
    assert!(
        out_grid.anchor.0.abs_diff(expected) <= 1,
        "{out_grid:?} vs {expected}"
    );

    // A later session finds the edit for the written file, still confirmed.
    let mut again = Analyzer::load(analysis, Some(lib.cache()), CancelToken::new())
        .expect("model")
        .analyze(&path)
        .expect("analysed")
        .record;
    let state = apply_saved(&mut again, &lib.edits());
    assert!(state.confirmed);
    assert!(again.grid.expect("grid").anchor.0.abs_diff(expected) <= 1);
}
