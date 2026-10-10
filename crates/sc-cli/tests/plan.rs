//! `sc-cli plan` end to end: the Action the app's table shows, decided from a synthetic tone at a
//! known level, in both loudness modes, as text and JSON. Loudness only (`--no-grid`), so no model
//! is needed; `SC_CACHE_DIR` points at a temp directory so the user's cache is never touched.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use sc_core::{AudioSpec, testsig};

/// A 5 s stereo 1 kHz tone at -20 dBFS: S-P95 and integrated loudness both read -20.0 LUFS and
/// the true peak is -20 dBTP.
fn tone_wav(dir: &Path) -> PathBuf {
    let path = dir.join("tone.wav");
    let buf = testsig::sine(AudioSpec::CD, 1000.0, 0.1, 5.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
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

fn plan(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::cargo_bin("sc-cli")
        .expect("binary")
        .env("SC_CACHE_DIR", dir.join("cache"))
        .env("SC_EDITS_DIR", dir.join("edits"))
        .arg("plan")
        .args(args)
        .arg("--no-grid")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    (
        output.status.success(),
        text + &String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn dj_mode_boosts_a_quiet_tone_to_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let wav = tone_wav(dir.path());
    let (ok, text) = plan(dir.path(), &[wav.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(
        text.contains("S-P95 -20.0 -> -11.0 LUFS  Gain +9.0 dB  Analysed"),
        "{text}"
    );
    assert!(
        text.contains("1 files: 1 analysed, 0 need review, 0 skipped, 0 failed"),
        "{text}"
    );
}

#[test]
fn streaming_mode_and_a_custom_target_change_the_gain() {
    let dir = tempfile::tempdir().unwrap();
    let wav = tone_wav(dir.path());
    let (ok, text) = plan(dir.path(), &[wav.to_str().unwrap(), "--mode", "streaming"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("I -20.0 -> -14.0 LUFS  Gain +6.0 dB"),
        "{text}"
    );
    let (ok, text) = plan(dir.path(), &[wav.to_str().unwrap(), "--target", "-30"]);
    assert!(ok, "{text}");
    assert!(text.contains("Gain -10.0 dB"), "{text}");
}

#[test]
fn a_low_ceiling_leaves_the_boost_short() {
    let dir = tempfile::tempdir().unwrap();
    let wav = tone_wav(dir.path());
    // 9 dB wanted, 5 dB of headroom under -15 dBTP.
    let (ok, text) = plan(dir.path(), &[wav.to_str().unwrap(), "--ceiling", "-15"]);
    assert!(ok, "{text}");
    assert!(text.contains("Gain +5.0 dB, short by 4.0 LU"), "{text}");
    assert!(text.contains("1 short of the target"), "{text}");
}

#[test]
fn json_carries_the_plan_as_the_app_receives_it() {
    let dir = tempfile::tempdir().unwrap();
    let wav = tone_wav(dir.path());
    let (ok, text) = plan(dir.path(), &[wav.to_str().unwrap(), "--json"]);
    assert!(ok, "{text}");
    let doc: serde_json::Value = serde_json::from_str(text.trim()).expect("one JSON document");
    assert_eq!(doc["plan"]["gain"]["type"], "gain");
    assert!((doc["plan"]["gain"]["gainDb"].as_f64().unwrap() - 9.0).abs() < 0.05);
    assert_eq!(doc["plan"]["status"], "analysed");
    assert_eq!(doc["plan"]["review"], serde_json::json!([]));
}

#[test]
fn mp3_moves_in_global_gain_steps() {
    let dir = tempfile::tempdir().unwrap();
    let mp3 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/lame-2s.mp3");
    let (ok, text) = plan(dir.path(), &[mp3.to_str().unwrap(), "--mode", "streaming"]);
    assert!(ok, "{text}");
    assert!(text.contains("Global-gain"), "{text}");
    assert!(text.contains("residual"), "{text}");
}

/// The model files, or `None` (the test is skipped) when they are not fetched.
fn models() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let ok =
        dir.join("mel_spectrogram.onnx").is_file() && dir.join("beat_this_small.onnx").is_file();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok.then_some(dir)
}

/// 0.4 s of silence, then 72 clicks at 124 BPM (bar 1 accented), 16-bit stereo 44.1 kHz.
fn click_wav(dir: &Path) -> PathBuf {
    let path = dir.join("click.wav");
    let clicks = testsig::click_track(AudioSpec::CD, sc_core::Bpm(124.0), 72, 15.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for _ in 0..2 * testsig::frames_for(AudioSpec::CD, 0.4) {
        w.write_sample(0_i16).expect("write");
    }
    for s in &clicks.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * 0.5 * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

#[test]
fn plan_prepare_prints_cut() {
    use sc_core::SampleIndex;
    use sc_core::analysis::{AnalysisSettings, GridEdit};
    use sc_engine::{Analyzer, CancelToken, save_edit};
    use sc_io::cache::Cache;
    use sc_io::edits::EditStore;

    let Some(models) = models() else { return };
    let dir = tempfile::tempdir().unwrap();
    let wav = click_wav(dir.path());
    let run = |args: &[&str]| {
        let out = Command::cargo_bin("sc-cli")
            .expect("binary")
            .env("SC_CACHE_DIR", dir.path().join("cache"))
            .env("SC_EDITS_DIR", dir.path().join("edits"))
            .env("SC_MODEL_DIR", &models)
            .arg("plan")
            .arg(&wav)
            .args(args)
            .output()
            .expect("run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // Bar 1 is placed on the first click, 0.400 s in (17,640 samples), as a user does in the
    // app; the click track's soft accents leave the analysed downbeat unsure.
    let settings = AnalysisSettings::default();
    let cache = Cache::open(dir.path().join("cache"));
    let record = Analyzer::load(settings.clone(), Some(cache), CancelToken::new())
        .unwrap()
        .analyze(&wav)
        .unwrap()
        .record;
    let edit = GridEdit {
        anchor: Some(SampleIndex(17_640)),
        ..GridEdit::default()
    };
    let store = EditStore::open(dir.path().join("edits"));
    save_edit(&store, &record, settings.bpm_range, &edit, true).unwrap();

    // The bar line at 0.400 s lands at the 5 ms lead: floor(17,640 - 220.5) = 17,419 frames,
    // 0.39 s, are cut (less than the 0.48 s beat).
    let text = run(&["--batch-mode", "prepare"]);
    assert!(
        text.contains("  Export (prepare): Gain +5.5 dB, Cut 0.39 s; tags BPM, "),
        "{text}"
    );
    let text = run(&["--batch-mode", "library"]);
    assert!(
        text.contains("  Export (library): Gain +5.5 dB, length kept; tags REPLAYGAIN_TRACK_GAIN"),
        "{text}"
    );
    // A 50 ms lead: 0.35 s.
    let text = run(&["--batch-mode", "prepare", "--lead-ms", "50"]);
    assert!(text.contains("Cut 0.35 s"), "{text}");
    // The file has no tag, so grid only has nothing to write.
    let text = run(&["--batch-mode", "prepare", "--grid-only"]);
    assert!(
        text.contains("XML only: grid only, and the file has no tag to write into"),
        "{text}"
    );
    let json = run(&["--batch-mode", "prepare", "--json"]);
    let doc: serde_json::Value = serde_json::from_str(json.trim()).expect("one document");
    assert_eq!(doc["export"]["type"], "write");
    assert_eq!(doc["export"]["plan"]["trimFrames"], 17_419);
    assert_eq!(doc["export"]["plan"]["cut"]["type"], "cut");
    // Without --batch-mode, nothing about export.
    let text = run(&[]);
    assert!(!text.contains("Export"), "{text}");
}

#[test]
fn export_flags_need_a_batch_mode_and_a_lead_in_range() {
    let dir = tempfile::tempdir().unwrap();
    let wav = tone_wav(dir.path());
    let (ok, text) = plan(dir.path(), &[wav.to_str().unwrap(), "--grid-only"]);
    assert!(!ok && text.contains("--batch-mode"), "{text}");
    let (ok, text) = plan(
        dir.path(),
        &[
            wav.to_str().unwrap(),
            "--batch-mode",
            "prepare",
            "--lead-ms",
            "80",
        ],
    );
    assert!(!ok && text.contains("outside 0 to 50 ms"), "{text}");
    // Loudness only: without a grid there is nothing to cut to.
    let (ok, text) = plan(
        dir.path(),
        &[wav.to_str().unwrap(), "--batch-mode", "prepare"],
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("  Export (prepare): Gain +9.0 dB, Not cut: no grid;"),
        "{text}"
    );
}

#[test]
fn mp3_is_left_to_the_xml() {
    let dir = tempfile::tempdir().unwrap();
    let mp3 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/lame-2s.mp3");
    let (ok, text) = plan(
        dir.path(),
        &[mp3.to_str().unwrap(), "--batch-mode", "library"],
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("  Export (library): XML only: MP3 (file writes arrive later)"),
        "{text}"
    );
}
