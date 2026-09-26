//! The per-file pipeline's cancellation and progress, and the grid replayed from the cached
//! evidence. Cancelling from the progress callback is deterministic: the callback runs on the
//! analysing thread before the next block is checked.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use sc_analysis::refit;
use sc_core::analysis::{AnalysisRecord, GridEdit};
use sc_core::{Bpm, Error};
use sc_engine::{Analyzer, CancelToken, Progress, Timings};
use sc_io::cache::Cache;

#[test]
fn cancel_during_decode_ends_cancelled_without_a_cache_entry() {
    let dir = tempfile::tempdir().unwrap();
    let wav = common::tone_wav(dir.path(), "long.wav", 30.0, 0.1);
    let cache = Cache::open(dir.path().join("cache"));
    let cancel = CancelToken::new();
    let mut analyzer =
        Analyzer::load(common::loudness_only(), Some(cache.clone()), cancel.clone()).unwrap();
    let token = cancel.clone();
    let progress: Progress = Arc::new(move |_| token.cancel());
    let result = analyzer.analyze_with(&wav, &mut Timings::default(), Some(&progress));
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    let (nfc, key) = Cache::key_for(&wav, &common::loudness_only()).unwrap();
    assert!(cache.get(&nfc, &key).is_none(), "no cache entry");
    assert!(!cache.entry_path(&nfc).exists());
}

#[test]
fn progress_rises_in_steps_of_at_least_one_percent_to_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let wav = common::tone_wav(dir.path(), "tone.wav", 20.0, 0.1);
    let mut analyzer = Analyzer::load(common::loudness_only(), None, CancelToken::new()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let progress: Progress = Arc::new(move |f| sink.lock().unwrap().push(f));
    analyzer
        .analyze_with(&wav, &mut Timings::default(), Some(&progress))
        .unwrap();
    let seen = seen.lock().unwrap().clone();
    assert!(seen.len() >= 10, "{} reports", seen.len());
    assert!(seen.len() <= 101, "{} reports", seen.len());
    assert!(seen.windows(2).all(|w| w[1] >= w[0] + 0.01), "{seen:?}");
    // Loudness only: decoding is the whole file.
    assert!(*seen.last().unwrap() > 0.99, "{seen:?}");
}

#[test]
fn a_cancelled_token_stops_before_opening_the_file() {
    let mut analyzer = Analyzer::load(common::loudness_only(), None, CancelToken::new()).unwrap();
    analyzer.cancel.cancel();
    let result = analyzer.analyze(std::path::Path::new("/nonexistent/file.wav"));
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}

/// The grid solved again from the cached evidence (after its JSON round trip), with no edit.
fn replayed(record: &AnalysisRecord, bpm_range: (Bpm, Bpm)) -> Option<sc_core::Grid> {
    let ctx = refit::Context {
        bpm_range,
        tags: &record.tags,
        sample_rate: record.spec.sample_rate,
    };
    refit::refit(record.evidence.as_ref()?, &ctx, &GridEdit::default()).map(|solved| solved.grid)
}

#[test]
fn an_empty_edit_replays_the_analysed_grid_from_the_cache() {
    if !common::have_models() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let settings = common::with_grid();
    let cache = Cache::open(dir.path().join("cache"));
    let mut analyzer =
        Analyzer::load(settings.clone(), Some(cache.clone()), CancelToken::new()).unwrap();
    for (bpm, lead) in [(124.0, 0.37), (93.5, 1.21), (172.0, 0.0)] {
        let wav = common::click_wav(dir.path(), bpm, 72, lead);
        let analysed = analyzer.analyze(&wav).unwrap().record;
        assert!(
            analysed.grid.is_some(),
            "{bpm}: {:?}",
            analysed.grid_skipped
        );
        let (nfc, key) = Cache::key_for(&wav, &settings).unwrap();
        let cached = cache.get(&nfc, &key).expect("cached");
        assert_eq!(cached, analysed, "{bpm}: the cache round trip is lossless");
        assert_eq!(
            replayed(&cached, settings.bpm_range),
            analysed.grid,
            "{bpm} BPM"
        );
    }
}

/// Every cached record in `SC_REPLAY_DIR` (the app's cache is
/// `~/Library/Caches/app.soundcheck.desktop/analysis`) replays to its grid. Records from before
/// schema 2 have no usable evidence and are counted, not compared. `SC_REPLAY_BPM_RANGE`
/// (default `70-180`) must be the range the records were analysed with.
#[test]
#[ignore = "reads a real cache directory named by SC_REPLAY_DIR"]
fn replay_matches_cached_records() {
    let Some(dir) = std::env::var_os("SC_REPLAY_DIR") else {
        eprintln!("skipped: set SC_REPLAY_DIR");
        return;
    };
    let range = std::env::var("SC_REPLAY_BPM_RANGE").unwrap_or_else(|_| "70-180".into());
    let (lo, hi) = range.split_once('-').expect("SC_REPLAY_BPM_RANGE is lo-hi");
    let bpm_range = (
        Bpm(lo.trim().parse().expect("lo")),
        Bpm(hi.trim().parse().expect("hi")),
    );
    let (mut compared, mut older, mut mismatched) = (0, 0, Vec::new());
    for entry in std::fs::read_dir(Path::new(&dir)).expect("read SC_REPLAY_DIR") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let Ok(record) = serde_json::from_value::<AnalysisRecord>(json["record"].clone()) else {
            older += 1;
            continue;
        };
        if record.evidence.is_none() {
            older += 1;
            continue;
        }
        compared += 1;
        if replayed(&record, bpm_range) != record.grid {
            mismatched.push(record.path);
        }
    }
    eprintln!(
        "replayed {compared} records ({older} older without evidence), {} mismatched",
        mismatched.len()
    );
    assert!(mismatched.is_empty(), "{mismatched:#?}");
}
