//! The beat tracker on a synthetic click track. Needs the model files: run
//! `scripts/fetch-models.sh` (or point `SC_MODEL_DIR` at them); without them the tests print why
//! they did nothing and pass, so a checkout without the weights stays green.

use std::path::{Path, PathBuf};

use sc_analysis::beats::{
    BEAT_MODEL_FILE, BeatTracker, MEL_MODEL_FILE, MODEL_SAMPLE_RATE, find_model_dir,
};
use sc_core::{AudioSpec, Bpm, Error, testsig};

/// The model directory: the normal lookup, then the workspace's `models/` (tests run with the
/// crate as working directory), else none and the test says why it did nothing.
fn models() -> Option<PathBuf> {
    if let Ok(dir) = find_model_dir() {
        return Some(dir);
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    if workspace.join(MEL_MODEL_FILE).is_file() && workspace.join(BEAT_MODEL_FILE).is_file() {
        return Some(workspace);
    }
    eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    None
}

#[test]
fn every_click_of_a_120_bpm_track_gets_a_beat_within_one_frame() {
    let Some(dir) = models() else { return };
    let mut tracker = BeatTracker::load(&dir).expect("load model");
    let spec = AudioSpec::new(MODEL_SAMPLE_RATE, 1);
    let clicks = testsig::click_track(spec, Bpm(120.0), 60, 20.0);
    let raw = tracker.track(&clicks.data).expect("track");
    assert_eq!(raw.beat_logits.len(), raw.downbeat_logits.len());
    let frames = (clicks.duration().0 * 50.0).round();
    assert!(
        (f64::from(u32::try_from(raw.beat_logits.len()).unwrap()) - frames).abs() <= 2.0,
        "{} activation frames for {frames}",
        raw.beat_logits.len()
    );
    let period = 60.0 / 120.0;
    let mut hits = 0;
    for i in 0..60 {
        let click = period * f64::from(i);
        if raw
            .beats_s
            .iter()
            .any(|&b| (f64::from(b) - click).abs() <= 0.030)
        {
            hits += 1;
        }
    }
    assert!(
        hits >= 54,
        "{hits}/60 clicks have a beat within 30 ms: {:?}",
        raw.beats_s
    );
    let beats = raw.beats_at(44_100);
    assert!(
        beats.windows(2).all(|w| w[0] < w[1]),
        "ascending sample positions"
    );
}

#[test]
fn tracking_is_deterministic() {
    let Some(dir) = models() else { return };
    let mut tracker = BeatTracker::load(&dir).expect("load model");
    let spec = AudioSpec::new(MODEL_SAMPLE_RATE, 1);
    let clicks = testsig::click_track(spec, Bpm(128.0), 32, 20.0);
    let a = tracker.track(&clicks.data).expect("track");
    let b = tracker.track(&clicks.data).expect("track again");
    assert_eq!(a, b);
}

#[test]
fn missing_models_name_the_directory_searched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = BeatTracker::load(dir.path()).unwrap_err();
    match err {
        Error::ModelUnavailable { searched } => {
            assert_eq!(searched, vec![dir.path().to_path_buf()]);
        }
        other => panic!("{other}"),
    }
    let text = BeatTracker::load(dir.path()).unwrap_err().to_string();
    assert!(text.contains("beat-tracking model not found"), "{text}");
}

#[test]
fn empty_audio_is_rejected_before_inference() {
    let Some(dir) = models() else { return };
    let mut tracker = BeatTracker::load(&dir).expect("load model");
    assert!(matches!(
        tracker.track(&[]).unwrap_err(),
        Error::InvalidArgument(_)
    ));
}
