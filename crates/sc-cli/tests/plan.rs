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
    assert!(doc["plan"]["review"].as_array().unwrap().is_empty());
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
