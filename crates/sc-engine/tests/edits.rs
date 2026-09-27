//! Saved grid edits reach the rows: the session applies them to each analysis before dropping
//! the evidence, a confirmed grid needs no review, the snapshot and a replan keep both, and a
//! changed file gets the analysis alone again. Needs the model files (skipped without).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sc_core::analysis::{AnalysisRecord, GridEdit};
use sc_core::ipc::{JobEvent, JobStage, RowAnalysis};
use sc_core::plan::{DecideSettings, Plan, ReviewReason};
use sc_core::{Bpm, SampleIndex};
use sc_engine::{
    Analyzer, BatchSettings, CancelToken, Session, collect_audio_files, probe_all, run_job,
};
use sc_io::cache::Cache;
use sc_io::edits::{EDIT_SCHEMA, EditStore, SavedEdit};

/// DJ settings whose BPM range leaves out the 124 BPM click track, so its row needs review.
fn decide_settings() -> DecideSettings {
    DecideSettings {
        bpm_range: (Bpm(130.0), Bpm(180.0)),
        ..DecideSettings::dj()
    }
}

fn analyse(wav: &Path, cache: &Cache) -> AnalysisRecord {
    let mut analyzer =
        Analyzer::load(common::with_grid(), Some(cache.clone()), CancelToken::new()).unwrap();
    analyzer.analyze(wav).unwrap().record
}

fn save(store: &EditStore, record: &AnalysisRecord, edit: GridEdit, confirmed: bool) {
    store
        .put(&SavedEdit {
            schema: EDIT_SCHEMA,
            path: record.path.clone(),
            size: record.size,
            mtime_ns: record.mtime_ns,
            bpm_range: common::with_grid().bpm_range,
            edit,
            confirmed,
        })
        .unwrap();
}

/// The session's row and plan for `wav` after one job.
fn row_and_plan(session: &Mutex<Session>, wav: &Path, cache: &Cache) -> (RowAnalysis, Plan) {
    let files: Vec<PathBuf> = collect_audio_files(&[wav.to_path_buf()]);
    let infos = probe_all(&files, 1);
    let ids: Vec<u32> = {
        let mut s = session.lock().unwrap();
        s.clear();
        s.add(files.into_iter().zip(infos).collect())
            .iter()
            .map(|e| e.file_id)
            .collect()
    };
    let job = session.lock().unwrap().next_job_id();
    let settings = BatchSettings {
        analysis: common::with_grid(),
        workers: 1,
        cache: Some(cache.clone()),
    };
    let mut events = Vec::new();
    run_job(
        session,
        job,
        &ids,
        &settings,
        &CancelToken::new(),
        &mut |e| {
            events.push(e);
        },
    );
    events
        .into_iter()
        .find_map(|e| match e {
            JobEvent::Analysed { row, plan, .. } => Some((*row, plan)),
            _ => None,
        })
        .expect("an analysed row")
}

#[test]
fn a_saved_edit_and_its_confirmation_reach_the_row_and_its_plan() {
    if !common::have_models() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = common::click_wav(dir.path(), 124.0, 72, 0.4);
    let cache = Cache::open(dir.path().join("cache"));
    let store = EditStore::open(dir.path().join("edits"));
    let analysed = analyse(&wav, &cache);
    let bar1 = analysed.grid.as_ref().expect("a grid").anchor;
    let session = Mutex::new(Session::new(decide_settings()).with_edits(store.clone()));

    // No edit: the analysis, needing review for its BPM.
    let (row, plan) = row_and_plan(&session, &wav, &cache);
    assert!(!row.edited && !row.confirmed);
    assert!(
        plan.review.contains(&ReviewReason::OutsideBpmRange),
        "{:?}",
        plan.review
    );

    // Beat 2 is beat 1, not confirmed yet: edited, still in review.
    let shift = GridEdit {
        downbeat_shift: 1,
        ..GridEdit::default()
    };
    save(&store, &analysed, shift.clone(), false);
    let (row, plan) = row_and_plan(&session, &wav, &cache);
    assert!(row.edited && !row.confirmed);
    assert_eq!(plan.status, JobStage::NeedsReview);
    let moved = row.grid.as_ref().unwrap().bar1.0 - bar1.to_seconds(44_100).0;
    assert!(
        (moved - 60.0 / 124.0).abs() <= 0.001,
        "bar 1 moved {moved:.4} s"
    );

    // Confirmed: out of review, the flags still on the row.
    save(&store, &analysed, shift, true);
    let (row, plan) = row_and_plan(&session, &wav, &cache);
    assert!(row.edited && row.confirmed);
    assert!(plan.review.is_empty(), "{:?}", plan.review);
    assert_eq!(plan.status, JobStage::Analysed);
    let s = session.lock().unwrap();
    let snapshot = &s.snapshot().rows[0];
    let restored = snapshot.row.as_ref().unwrap();
    assert!(restored.edited && restored.confirmed);
    assert_eq!(snapshot.plan.as_ref().unwrap().status, JobStage::Analysed);
    drop(s);
    let replan = session
        .lock()
        .unwrap()
        .set_settings(DecideSettings {
            target: sc_core::Lufs(-8.0),
            ..decide_settings()
        })
        .unwrap();
    assert_eq!(replan.plans[0].plan.status, JobStage::Analysed);
}

#[test]
fn a_changed_file_gets_the_analysis_alone_again() {
    if !common::have_models() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = common::click_wav(dir.path(), 124.0, 72, 0.4);
    let cache = Cache::open(dir.path().join("cache"));
    let store = EditStore::open(dir.path().join("edits"));
    let analysed = analyse(&wav, &cache);
    save(
        &store,
        &analysed,
        GridEdit {
            anchor: Some(SampleIndex(analysed.grid.as_ref().unwrap().anchor.0 + 441)),
            ..GridEdit::default()
        },
        true,
    );
    // Rewrite the file with one more beat: new size and modification time.
    std::thread::sleep(std::time::Duration::from_millis(20));
    common::click_wav(dir.path(), 124.0, 73, 0.4);
    let session = Mutex::new(Session::new(decide_settings()).with_edits(store));
    let (row, plan) = row_and_plan(&session, &wav, &cache);
    assert!(!row.edited && !row.confirmed);
    assert_eq!(plan.status, JobStage::NeedsReview);
}
