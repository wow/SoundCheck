//! Saved grid edits reach the rows: a job applies them before the session stores each analysis,
//! a confirmed grid needs no review, the snapshot and a replan keep both, an edit keeps the BPM
//! range it was made under, survives a rewrite of the same audio (a tag write), is dropped for
//! other audio, and a confirmation counts only for the grid that was confirmed. Needs the model
//! files (skipped without).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sc_core::analysis::{AnalysisRecord, GridEdit};
use sc_core::ipc::{JobEvent, JobStage, RowAnalysis};
use sc_core::plan::{DecideSettings, Plan, ReviewReason};
use sc_core::{Bpm, SampleIndex};
use sc_engine::{
    Analyzer, BatchSettings, CancelToken, EditState, Session, collect_audio_files, probe_all,
    run_job, save_edit,
};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;

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

fn save(store: &EditStore, record: &AnalysisRecord, edit: &GridEdit, confirmed: bool) {
    save_edit(
        store,
        record,
        common::with_grid().bpm_range,
        edit,
        confirmed,
    )
    .unwrap();
}

/// The session's row and plan for `wav` after one job analysing with `analysis`.
fn row_and_plan(
    session: &Mutex<Session>,
    wav: &Path,
    cache: &Cache,
    analysis: sc_core::analysis::AnalysisSettings,
) -> (RowAnalysis, Plan) {
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
        analysis,
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

struct Fixture {
    _dir: tempfile::TempDir,
    wav: PathBuf,
    cache: Cache,
    store: EditStore,
    analysed: AnalysisRecord,
    session: Mutex<Session>,
}

fn fixture() -> Option<Fixture> {
    if !common::have_models() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = common::click_wav(dir.path(), 124.0, 72, 0.4);
    let cache = Cache::open(dir.path().join("cache"));
    let store = EditStore::open(dir.path().join("edits"));
    let analysed = analyse(&wav, &cache);
    let session = Mutex::new(Session::new(decide_settings()).with_edits(store.clone()));
    Some(Fixture {
        _dir: dir,
        wav,
        cache,
        store,
        analysed,
        session,
    })
}

impl Fixture {
    fn row(&self) -> (RowAnalysis, Plan) {
        row_and_plan(&self.session, &self.wav, &self.cache, common::with_grid())
    }
}

#[test]
fn a_saved_edit_and_its_confirmation_reach_the_row_and_its_plan() {
    let Some(f) = fixture() else { return };
    let bar1 = f.analysed.grid.as_ref().expect("a grid").anchor;

    // No edit: the analysis, needing review for its BPM.
    let (row, plan) = f.row();
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
    save(&f.store, &f.analysed, &shift, false);
    let (row, plan) = f.row();
    assert!(row.edited && !row.confirmed);
    assert_eq!(plan.status, JobStage::NeedsReview);
    let moved = row.grid.as_ref().unwrap().bar1.0 - bar1.to_seconds(44_100).0;
    assert!(
        (moved - 60.0 / 124.0).abs() <= 0.001,
        "bar 1 moved {moved:.4} s"
    );

    // Confirmed: out of review, the flags still on the row.
    save(&f.store, &f.analysed, &shift, true);
    let (row, plan) = f.row();
    assert!(row.edited && row.confirmed);
    assert!(plan.review.is_empty(), "{:?}", plan.review);
    assert_eq!(plan.status, JobStage::Analysed);
    let snapshot = f.session.lock().unwrap().snapshot();
    let restored = snapshot.rows[0].row.as_ref().unwrap();
    assert!(restored.edited && restored.confirmed);
    assert_eq!(
        snapshot.rows[0].plan.as_ref().unwrap().status,
        JobStage::Analysed
    );
    let replan = f
        .session
        .lock()
        .unwrap()
        .set_settings(DecideSettings {
            target: sc_core::Lufs(-8.0),
            ..decide_settings()
        })
        .unwrap();
    assert_eq!(replan.plans[0].plan.status, JobStage::Analysed);

    // An empty, unconfirmed save removes the edit.
    let state = save_edit(
        &f.store,
        &f.analysed,
        common::with_grid().bpm_range,
        &GridEdit::default(),
        false,
    )
    .unwrap();
    assert_eq!(state, EditState::default());
    assert!(f.store.get(&f.analysed.path).is_none());
}

#[test]
fn an_edit_keeps_the_bpm_range_it_was_made_under() {
    let Some(f) = fixture() else { return };
    // Confirmed at 124 under 70-180; the app's range then becomes 40-100, where the analysis
    // takes 62.
    save(&f.store, &f.analysed, &GridEdit::default(), true);
    let narrow = sc_core::analysis::AnalysisSettings {
        bpm_range: (Bpm(40.0), Bpm(100.0)),
        ..common::with_grid()
    };
    let (row, plan) = row_and_plan(&f.session, &f.wav, &f.cache, narrow);
    let grid = row.grid.as_ref().unwrap();
    assert!((grid.bpm.0 - 124.0).abs() < 0.01, "{}", grid.bpm.0);
    assert!(row.edited, "differs from the 62 BPM analysis");
    assert!(row.confirmed);
    assert_eq!(plan.status, JobStage::Analysed);
}

#[test]
fn a_rewrite_of_the_same_audio_keeps_the_edit_and_other_audio_drops_it() {
    let Some(f) = fixture() else { return };
    save(
        &f.store,
        &f.analysed,
        &GridEdit {
            anchor: Some(SampleIndex(
                f.analysed.grid.as_ref().unwrap().anchor.0 + 441,
            )),
            ..GridEdit::default()
        },
        true,
    );
    // A tag write: new modification time, same audio.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&f.wav, std::fs::read(&f.wav).unwrap()).unwrap();
    let (row, plan) = f.row();
    assert!(row.edited && row.confirmed, "{row:?}");
    assert_eq!(plan.status, JobStage::Analysed);
    // Other audio: one more beat.
    common::click_wav(f.wav.parent().unwrap(), 124.0, 73, 0.4);
    let (row, plan) = f.row();
    assert!(!row.edited && !row.confirmed);
    assert_eq!(plan.status, JobStage::NeedsReview);
}

#[test]
fn a_confirmation_counts_only_for_the_grid_that_was_confirmed() {
    let Some(f) = fixture() else { return };
    save(&f.store, &f.analysed, &GridEdit::default(), true);
    // The pinned grid no longer matches what the edit gives (as after a new beat model).
    let mut saved = f.store.get(&f.analysed.path).unwrap();
    let pin = saved.grid.as_mut().unwrap();
    pin.anchor = SampleIndex(pin.anchor.0 + 441);
    f.store.put(&saved).unwrap();
    let (row, plan) = f.row();
    assert!(!row.confirmed, "{row:?}");
    assert_eq!(plan.status, JobStage::NeedsReview);
}
