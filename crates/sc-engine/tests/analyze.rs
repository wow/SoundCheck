//! The per-file pipeline's cancellation and progress. Cancelling from the progress callback is
//! deterministic: the callback runs on the analysing thread before the next block is checked.

mod common;

use std::sync::{Arc, Mutex};

use sc_core::Error;
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
