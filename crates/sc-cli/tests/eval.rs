//! `sc-cli eval` on synthetic files with labels written next to them.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use sc_core::{AudioSpec, testsig};

fn models() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let ok =
        dir.join("mel_spectrogram.onnx").is_file() && dir.join("beat_this_small.onnx").is_file();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok.then_some(dir)
}

/// `lead` seconds of silence then `buf`, as 16-bit WAV.
fn write_wav(path: &Path, buf: &sc_core::AudioBuffer, lead: f64, gain: f64) {
    let spec = hound::WavSpec {
        channels: buf.spec.channels,
        sample_rate: buf.spec.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    let lead_samples = usize::from(buf.spec.channels) * testsig::frames_for(buf.spec, lead);
    for _ in 0..lead_samples {
        w.write_sample(0_i16).expect("write");
    }
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * gain * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
}

fn run_eval(dir: &Path, labels: &str, extra: &[&str]) -> (bool, String) {
    let csv = dir.join("labels.csv");
    std::fs::write(&csv, labels).unwrap();
    let mut cmd = Command::cargo_bin("sc-cli").expect("binary");
    if let Some(m) = models() {
        cmd.env("SC_MODEL_DIR", m);
    }
    let out = cmd
        .args([
            "eval",
            "--labels",
            csv.to_str().unwrap(),
            "--dir",
            dir.to_str().unwrap(),
        ])
        .args(extra)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

#[test]
fn loudness_is_scored_against_the_reference_without_a_model() {
    let dir = tempfile::tempdir().unwrap();
    let tones = dir.path().join("tones");
    std::fs::create_dir(&tones).unwrap();
    // A 5 s stereo 1 kHz tone at 0.1 peak reads -20.0 LUFS.
    write_wav(
        &tones.join("tone.wav"),
        &testsig::sine(AudioSpec::CD, 1000.0, 0.1, 5.0),
        0.0,
        1.0,
    );
    let (ok, out) = run_eval(
        dir.path(),
        "file,ffmpeg_i_lufs\ntone.wav,-20.0\nmissing.wav,-10\n",
        &["--no-grid"],
    );
    assert!(ok, "{out}");
    assert!(
        out.contains("integrated within 0.1 LU of ffmpeg: 1/1"),
        "{out}"
    );
    assert!(
        out.contains("missing.wav") && out.contains("not found"),
        "{out}"
    );
    assert!(out.contains("analysed 1/2"), "{out}");
}

#[test]
fn a_labelled_click_track_scores_bpm_meter_and_bar_one() {
    let Some(_) = models() else { return };
    let dir = tempfile::tempdir().unwrap();
    let clicks = testsig::click_track(AudioSpec::CD, sc_core::Bpm(120.0), 120, 15.0);
    write_wav(&dir.path().join("click.wav"), &clicks, 0.5, 0.5);
    let (ok, out) = run_eval(
        dir.path(),
        "file,bpm,bar1_s,meter,grouping\nclick.wav,120.00,0.500,4/4,\n",
        &[],
    );
    assert!(ok, "{out}");
    assert!(out.contains("bpm within 0.02: 1/1"), "{out}");
    assert!(out.contains("meter and grouping: 1/1"), "{out}");
    assert!(out.contains("bar 1 within 15 ms: 1/1"), "{out}");
}

#[test]
fn a_labels_file_without_a_file_column_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out) = run_eval(dir.path(), "bpm\n120\n", &["--no-grid"]);
    assert!(!ok);
    assert!(out.contains("file"), "{out}");
}
