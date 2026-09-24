//! `sc-cli analyze --json` on a synthetic WAV prints the expected fields.

use assert_cmd::Command;
use sc_core::{AudioSpec, testsig};

fn write_sine_wav(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("sine.wav");
    let buf = testsig::sine(AudioSpec::CD, 1000.0, 0.5, 0.2);
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

#[test]
fn analyze_json_reports_spec_frames_and_peak() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav = write_sine_wav(dir.path());
    let output = Command::cargo_bin("sc-cli")
        .expect("binary")
        .args(["analyze", wav.to_str().expect("utf-8 path"), "--json"])
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(report["schema"], 1);
    assert_eq!(report["spec"]["sampleRate"], 44_100);
    assert_eq!(report["spec"]["channels"], 2);
    assert_eq!(report["frames"], 8_820);
    let peak = report["samplePeakDbfs"].as_f64().expect("number");
    assert!((peak + 6.02).abs() < 0.05, "peak {peak}");
}

#[test]
fn version_prints_semver_with_revision_and_date() {
    let output = Command::cargo_bin("sc-cli")
        .expect("binary")
        .arg("--version")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.starts_with("sc-cli 0."), "{text}");
    assert!(text.contains('(') && text.contains(", 20"), "{text}");
}

#[test]
fn missing_file_fails_with_a_message() {
    Command::cargo_bin("sc-cli")
        .expect("binary")
        .args(["analyze", "/nonexistent.wav"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("nonexistent.wav"));
}
