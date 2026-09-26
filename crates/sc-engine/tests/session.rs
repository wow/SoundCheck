//! The app session: stable ids, one job's IPC events end to end, replanning on a settings change,
//! and the calibration median. Loudness only, so no model is needed.

mod common;

use std::path::PathBuf;
use std::sync::Mutex;

use sc_core::Lufs;
use sc_core::ipc::{IpcErrorKind, JobEvent, JobStage};
use sc_core::plan::{DecideSettings, GainPlan};
use sc_engine::{BatchSettings, CancelToken, Session, collect_audio_files, probe_all, run_job};

fn add(session: &mut Session, paths: &[PathBuf]) -> Vec<u32> {
    let files = collect_audio_files(paths);
    let infos = probe_all(&files, 2);
    session
        .add(files.into_iter().zip(infos).collect())
        .iter()
        .map(|e| e.file_id)
        .collect()
}

fn settings() -> BatchSettings {
    BatchSettings {
        analysis: common::loudness_only(),
        workers: 2,
        cache: None,
    }
}

#[test]
fn adding_the_same_file_twice_keeps_one_row() {
    let dir = tempfile::tempdir().unwrap();
    let a = common::tone_wav(dir.path(), "a.wav", 1.0, 0.1);
    let mut session = Session::new(DecideSettings::dj());
    assert_eq!(add(&mut session, std::slice::from_ref(&a)), vec![1]);
    assert_eq!(
        add(&mut session, &[dir.path().to_path_buf()]),
        Vec::<u32>::new()
    );
    let b = common::tone_wav(dir.path(), "b.wav", 1.0, 0.1);
    assert_eq!(add(&mut session, &[b]), vec![2]);
    assert_eq!(session.len(), 2);
}

#[test]
fn a_job_streams_rows_with_plans_and_ends_with_finished() {
    let dir = tempfile::tempdir().unwrap();
    // 5 s at -20 dBFS: S-P95 -20 LUFS, so the DJ plan is +9 dB.
    common::tone_wav(dir.path(), "1 quiet.wav", 5.0, 0.1);
    common::tone_wav(dir.path(), "2 loud.wav", 5.0, 0.5);
    common::corrupt_wav(dir.path(), "3 broken.wav");
    let session = Mutex::new(Session::new(DecideSettings::dj()));
    let ids = add(&mut session.lock().unwrap(), &[dir.path().to_path_buf()]);
    assert_eq!(ids, vec![1, 2, 3]);
    let job = session.lock().unwrap().next_job_id();
    let mut events = Vec::new();
    run_job(
        &session,
        job,
        &ids,
        &settings(),
        &CancelToken::new(),
        &mut |e| events.push(e),
    );

    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished {
            cancelled: false,
            ..
        })
    ));
    let analysed: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Analysed {
                file_id, row, plan, ..
            } => Some((*file_id, row, plan)),
            _ => None,
        })
        .collect();
    assert_eq!(analysed.len(), 2);
    let (_, row, plan) = analysed.iter().find(|(id, ..)| *id == 1).unwrap();
    assert!(
        (row.short_term_p95.unwrap().0 + 20.0).abs() < 0.1,
        "{row:?}"
    );
    assert!(row.grid.is_none() && !row.cached);
    let Some(GainPlan::Gain { gain_db, .. }) = plan.gain else {
        panic!("{plan:?}")
    };
    assert!((gain_db - 9.0).abs() < 0.1, "{gain_db}");
    assert_eq!(plan.status, JobStage::Analysed);
    let failed = events.iter().find_map(|e| match e {
        JobEvent::Failed { file_id, error, .. } => Some((*file_id, error)),
        _ => None,
    });
    let (file_id, error) = failed.expect("the broken file fails");
    assert_eq!(file_id, 3);
    assert_eq!(error.file_id, Some(3));
    assert_ne!(error.kind, IpcErrorKind::Cancelled);
    assert!(events.iter().all(|e| match e {
        JobEvent::Started { job_id, .. }
        | JobEvent::Progress { job_id, .. }
        | JobEvent::Analysed { job_id, .. }
        | JobEvent::Failed { job_id, .. }
        | JobEvent::Cancelled { job_id, .. }
        | JobEvent::Batch { job_id, .. }
        | JobEvent::Aborted { job_id, .. }
        | JobEvent::Finished { job_id, .. } => *job_id == job,
    }));

    // Replanning needs no analysis: streaming aligns integrated loudness at -14.
    let plans = session
        .lock()
        .unwrap()
        .set_settings(DecideSettings::streaming());
    assert_eq!(plans.len(), 2);
    let quiet = plans.iter().find(|p| p.file_id == 1).unwrap();
    let Some(GainPlan::Gain { gain_db, .. }) = quiet.plan.gain else {
        panic!()
    };
    assert!((gain_db - 6.0).abs() < 0.1, "{gain_db}");

    // Calibration: the median S-P95 of the two analysed rows.
    let median = session.lock().unwrap().median_short_term_p95().unwrap();
    let loud = -20.0 + 20.0 * 5f64.log10();
    assert!(
        (median.0 - f64::midpoint(-20.0, loud)).abs() < 0.1,
        "{median:?}"
    );
}

#[test]
fn a_cancelled_job_ends_every_row_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    common::tone_wav(dir.path(), "a.wav", 2.0, 0.1);
    common::tone_wav(dir.path(), "b.wav", 2.0, 0.1);
    let session = Mutex::new(Session::new(DecideSettings::dj()));
    let ids = add(&mut session.lock().unwrap(), &[dir.path().to_path_buf()]);
    let cancel = CancelToken::new();
    cancel.cancel();
    let mut events = Vec::new();
    run_job(&session, 7, &ids, &settings(), &cancel, &mut |e| {
        events.push(e);
    });
    let cancelled = events
        .iter()
        .filter(|e| matches!(e, JobEvent::Cancelled { .. }))
        .count();
    assert_eq!(cancelled, 2);
    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished {
            job_id: 7,
            cancelled: true
        })
    ));
    assert!(
        session.lock().unwrap().plan(ids[0]).is_none(),
        "nothing stored"
    );
}

#[test]
fn a_job_without_its_model_is_aborted_then_finished() {
    if sc_analysis::beats::find_model_dir().is_ok() {
        eprintln!("skipped: the model files are installed on this machine");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::tone_wav(dir.path(), "a.wav", 2.0, 0.1);
    let session = Mutex::new(Session::new(DecideSettings::dj()));
    let ids = add(&mut session.lock().unwrap(), &[dir.path().to_path_buf()]);
    let mut s = settings();
    s.analysis.grid = true;
    let mut events = Vec::new();
    run_job(&session, 1, &ids, &s, &CancelToken::new(), &mut |e| {
        events.push(e);
    });
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(
        &events[0],
        JobEvent::Aborted { error, .. } if error.kind == IpcErrorKind::ModelUnavailable
    ));
    assert!(matches!(
        events[1],
        JobEvent::Finished {
            cancelled: false,
            ..
        }
    ));
}

#[test]
fn median_of_nothing_is_none() {
    assert_eq!(
        Session::new(DecideSettings::dj()).median_short_term_p95(),
        None::<Lufs>
    );
}
