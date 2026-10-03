//! Unit tests of the private parts of `src-tauri/src/shell.rs`.
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use sc_core::analysis::{AnalysisSettings, Model};
use sc_core::plan::GainPlan;
use sc_core::{AudioSpec, Bpm, testsig};

use super::*;

fn tone_wav(dir: &Path, name: &str, amp: f32) {
    let buf = testsig::sine(AudioSpec::CD, 1000.0, amp, 5.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(dir.join(name), spec).unwrap();
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * f64::from(i16::MAX)).round() as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

fn loudness_only() -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: false,
        model: Model::Small,
    }
}

/// Collects a job's events until `finished`.
fn events_until_finished(rx: &mpsc::Receiver<JobEvent>) -> Vec<JobEvent> {
    let mut events = Vec::new();
    loop {
        let e = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the job ends");
        let done = matches!(e, JobEvent::Finished { .. });
        events.push(e);
        if done {
            return events;
        }
    }
}

#[test]
fn a_dropped_folder_becomes_rows_then_analysed_rows_with_plans() {
    let dir = tempfile::tempdir().unwrap();
    tone_wav(dir.path(), "a.wav", 0.1);
    tone_wav(dir.path(), "b.wav", 0.3);
    let shell = Shell::new(None, None, 2);
    let rows = shell.expand(vec![dir.path().display().to_string()]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].info.sample_rate, Some(44_100));
    assert!(
        shell
            .expand(vec![dir.path().display().to_string()])
            .is_empty(),
        "no duplicates"
    );

    let (tx, rx) = mpsc::channel();
    let req = AnalyzeRequest {
        file_ids: rows.iter().map(|r| r.file_id).collect(),
        analysis: loudness_only(),
    };
    let job = shell.start(req, move |e| tx.send(e).unwrap()).unwrap();
    let events = events_until_finished(&rx);
    assert!(
        matches!(events.last(), Some(JobEvent::Finished { job_id, cancelled: false }) if *job_id == job)
    );
    let analysed = events
        .iter()
        .filter(|e| matches!(e, JobEvent::Analysed { .. }))
        .count();
    assert_eq!(analysed, 2);
    assert!(!shell.cancel(job), "a finished job cannot be cancelled");

    let replan = shell
        .set_decide_settings(DecideSettings::streaming())
        .unwrap();
    assert_eq!(replan.plans.len(), 2);
    assert!(
        replan
            .plans
            .iter()
            .all(|p| matches!(p.plan.gain, Some(GainPlan::Gain { .. })))
    );
    assert!(shell.calibration_target().is_some());
    let snapshot = shell.restore();
    assert_eq!(snapshot.rows.len(), 2);
    assert!(snapshot.rows.iter().all(|r| r.plan.is_some()));
}

#[test]
fn cancel_stops_a_running_job() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..6 {
        tone_wav(dir.path(), &format!("{i}.wav"), 0.1);
    }
    let shell = Shell::new(None, None, 1);
    let rows = shell.expand(vec![dir.path().display().to_string()]);
    let (tx, rx) = mpsc::channel();
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    let req = AnalyzeRequest {
        file_ids: rows.iter().map(|r| r.file_id).collect(),
        analysis: loudness_only(),
    };
    // Hold the job at its first event until Cancel has been pressed, so the cancel lands
    // while at most the first file is in flight.
    let mut gate = Some(gate_rx);
    let job = shell
        .start(req, move |e| {
            let _ = tx.send(e);
            if let Some(gate) = gate.take() {
                let _ = gate.recv();
            }
        })
        .unwrap();
    let started = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(matches!(started, JobEvent::Started { .. }), "{started:?}");
    assert!(shell.cancel(job), "the job is running");
    gate_tx.send(()).unwrap();
    let events = events_until_finished(&rx);
    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished {
            cancelled: true,
            ..
        })
    ));
    let count = |f: fn(&JobEvent) -> bool| events.iter().filter(|e| f(e)).count();
    let cancelled = count(|e| matches!(e, JobEvent::Cancelled { .. }));
    let analysed = count(|e| matches!(e, JobEvent::Analysed { .. }));
    assert_eq!(cancelled + analysed, 6, "every row ends");
    assert!(cancelled >= 4, "{cancelled} cancelled");
}

#[test]
fn out_of_range_settings_are_refused() {
    let shell = Shell::new(None, None, 1);
    let bad = DecideSettings {
        ceiling: sc_core::DbTp(0.5),
        ..DecideSettings::dj()
    };
    let err = shell.set_decide_settings(bad).unwrap_err();
    assert_eq!(err.kind, sc_core::ipc::IpcErrorKind::InvalidArgument);
    let mut analysis = loudness_only();
    analysis.bpm_range = (Bpm(180.0), Bpm(70.0));
    let req = AnalyzeRequest {
        file_ids: vec![],
        analysis,
    };
    assert!(shell.start(req, |_| {}).is_err());
}

#[test]
fn a_cleared_list_takes_the_same_files_again() {
    let dir = tempfile::tempdir().unwrap();
    tone_wav(dir.path(), "a.wav", 0.1);
    let shell = Shell::new(None, None, 1);
    let first = shell.expand(vec![dir.path().display().to_string()]);
    assert_eq!(first.len(), 1);
    shell.clear();
    assert_eq!(shell.restore().rows, Vec::new());
    let again = shell.expand(vec![dir.path().display().to_string()]);
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].file_id, first[0].file_id, "ids are not reused");
}

#[test]
fn cancelling_an_unknown_job_says_so() {
    assert!(!Shell::new(None, None, 1).cancel(42));
}
