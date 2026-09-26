//! `run_batch`: the event sequence per file, a failing file, cancellation, determinism across
//! worker counts, and a missing model. Loudness-only settings keep these runnable without the
//! model files; the grid path is covered by `sc-cli`'s tests and the CLI comparison runs.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use sc_core::Error;
use sc_core::analysis::AnalysisSettings;
use sc_engine::{BatchFile, BatchSettings, CancelToken, EngineEvent, run_batch};
use sc_io::cache::Cache;

fn batch(paths: &[PathBuf]) -> Vec<BatchFile> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| BatchFile {
            file_id: u32::try_from(i).unwrap() + 100,
            path: p.clone(),
            duration_hint: None,
        })
        .collect()
}

fn settings(workers: usize, cache: Option<Cache>) -> BatchSettings {
    BatchSettings {
        analysis: common::loudness_only(),
        workers,
        cache,
    }
}

/// Runs a batch and returns its events.
fn run(files: &[BatchFile], s: &BatchSettings, cancel: &CancelToken) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    run_batch(files, s, cancel, &mut |e| events.push(e)).expect("batch starts");
    events
}

fn file_id(event: &EngineEvent) -> Option<u32> {
    match event {
        EngineEvent::Started { file_id }
        | EngineEvent::Progress { file_id, .. }
        | EngineEvent::Analysed { file_id, .. }
        | EngineEvent::Failed { file_id, .. }
        | EngineEvent::Cancelled { file_id } => Some(*file_id),
        EngineEvent::Batch(_) => None,
    }
}

fn is_terminal(event: &EngineEvent) -> bool {
    matches!(
        event,
        EngineEvent::Analysed { .. } | EngineEvent::Failed { .. } | EngineEvent::Cancelled { .. }
    )
}

#[test]
fn every_file_starts_reports_progress_and_ends_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..5)
        .map(|i| common::tone_wav(dir.path(), &format!("t{i}.wav"), 3.0 + f64::from(i), 0.1))
        .collect();
    let files = batch(&paths);
    let events = run(&files, &settings(2, None), &CancelToken::new());
    for file in &files {
        let mine: Vec<&EngineEvent> = events
            .iter()
            .filter(|e| file_id(e) == Some(file.file_id))
            .collect();
        assert!(matches!(mine[0], EngineEvent::Started { .. }), "{mine:?}");
        assert_eq!(
            mine.iter().filter(|e| is_terminal(e)).count(),
            1,
            "{mine:?}"
        );
        assert!(matches!(mine.last().unwrap(), EngineEvent::Analysed { .. }));
        let fractions: Vec<f32> = mine
            .iter()
            .filter_map(|e| match e {
                EngineEvent::Progress { fraction, .. } => Some(*fraction),
                _ => None,
            })
            .collect();
        assert!(fractions.windows(2).all(|w| w[0] <= w[1]), "{fractions:?}");
    }
    let EngineEvent::Batch(last) = events.last().unwrap() else {
        panic!("the batch ends with a summary");
    };
    assert_eq!((last.done, last.total), (5, 5));
    assert!(last.realtime_x.is_some_and(|x| x > 1.0));
}

#[test]
fn a_corrupt_file_fails_alone() {
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![
        common::tone_wav(dir.path(), "a.wav", 2.0, 0.1),
        common::corrupt_wav(dir.path(), "broken.wav"),
        common::tone_wav(dir.path(), "c.wav", 2.0, 0.1),
    ];
    let files = batch(&paths);
    let mut events = Vec::new();
    let summary = run_batch(&files, &settings(2, None), &CancelToken::new(), &mut |e| {
        events.push(e);
    })
    .unwrap();
    assert_eq!(
        (summary.analysed, summary.failed, summary.cancelled),
        (2, 1, 0)
    );
    assert!(events.iter().any(|e| matches!(
        e,
        EngineEvent::Failed { file_id: 101, error } if !matches!(error, Error::Cancelled)
    )));
}

#[test]
fn a_cancelled_batch_ends_every_file_and_caches_only_finished_ones() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..5)
        .map(|i| common::tone_wav(dir.path(), &format!("long{i}.wav"), 60.0, 0.1))
        .collect();
    let files = batch(&paths);
    let cache = Cache::open(dir.path().join("cache"));
    let cancel = CancelToken::new();
    let mut events = Vec::new();
    let token = cancel.clone();
    let summary = run_batch(
        &files,
        &settings(1, Some(cache.clone())),
        &cancel,
        &mut |e| {
            // Cancel as soon as the first file is done.
            if matches!(e, EngineEvent::Analysed { .. }) {
                token.cancel();
            }
            events.push(e);
        },
    )
    .unwrap();
    assert!(summary.was_cancelled);
    assert_eq!(summary.analysed + summary.cancelled, 5, "{summary:?}");
    assert!(summary.cancelled >= 3, "{summary:?}");
    let mut terminal: BTreeMap<u32, usize> = BTreeMap::new();
    for e in events.iter().filter(|e| is_terminal(e)) {
        *terminal.entry(file_id(e).unwrap()).or_default() += 1;
    }
    assert_eq!(terminal.len(), 5);
    assert!(terminal.values().all(|&n| n == 1), "{terminal:?}");
    for file in &files {
        let analysed = events.iter().any(
            |e| matches!(e, EngineEvent::Analysed { file_id, .. } if *file_id == file.file_id),
        );
        let (nfc, key) = Cache::key_for(&file.path, &common::loudness_only()).unwrap();
        assert_eq!(
            cache.get(&nfc, &key).is_some(),
            analysed,
            "{}",
            file.path.display()
        );
    }
}

#[test]
fn a_batch_cancelled_before_it_starts_analyses_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![common::tone_wav(dir.path(), "a.wav", 2.0, 0.1)];
    let cancel = CancelToken::new();
    cancel.cancel();
    let events = run(&batch(&paths), &settings(2, None), &cancel);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, EngineEvent::Cancelled { file_id: 100 }))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EngineEvent::Analysed { .. }))
    );
}

#[test]
fn one_and_four_workers_give_identical_records() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..6)
        .map(|i| {
            common::tone_wav(
                dir.path(),
                &format!("t{i}.wav"),
                2.0,
                0.05 * f32::from(1 + u8::try_from(i).unwrap()),
            )
        })
        .collect();
    let files = batch(&paths);
    let records = |workers| {
        run(&files, &settings(workers, None), &CancelToken::new())
            .into_iter()
            .filter_map(|e| match e {
                EngineEvent::Analysed { file_id, report } => {
                    Some((file_id, serde_json::to_value(&report.record).unwrap()))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>()
    };
    let one = records(1);
    assert_eq!(one.len(), 6);
    assert_eq!(one, records(4));
}

#[test]
fn a_missing_model_fails_before_any_file() {
    if sc_analysis::beats::find_model_dir().is_ok() {
        eprintln!("skipped: the model files are installed on this machine");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![common::tone_wav(dir.path(), "a.wav", 2.0, 0.1)];
    let s = BatchSettings {
        analysis: AnalysisSettings {
            grid: true,
            ..common::loudness_only()
        },
        workers: 2,
        cache: None,
    };
    let mut events = 0;
    let result = run_batch(&batch(&paths), &s, &CancelToken::new(), &mut |_| {
        events += 1;
    });
    assert!(
        matches!(result, Err(Error::ModelUnavailable { .. })),
        "{result:?}"
    );
    assert_eq!(events, 0);
}
