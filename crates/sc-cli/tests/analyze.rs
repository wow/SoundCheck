//! `sc-cli analyze` end to end on a synthetic WAV and the committed fixtures: JSON fields, the
//! cache's hit / written / bypassed states, error reports with exit code 2, and `sc-cli cache`.
//! Every run points `SC_CACHE_DIR` at a temp directory so the user's cache is never touched.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use sc_core::{AudioSpec, testsig};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

/// A 5 s stereo 1 kHz tone at -20 dBFS: long enough for gated integrated loudness.
fn write_tone_wav(dir: &Path) -> PathBuf {
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

fn sc_cli(cache_dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("sc-cli").expect("binary");
    cmd.env("SC_CACHE_DIR", cache_dir);
    cmd
}

fn analyze_json(cache_dir: &Path, args: &[&str]) -> (bool, serde_json::Value, String) {
    let output = sc_cli(cache_dir)
        .arg("analyze")
        .args(args)
        .arg("--json")
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let doc: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    (
        output.status.success(),
        doc,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn analyze_json_reports_loudness_and_writes_the_cache() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let wav = write_tone_wav(dir.path());
    let (ok, doc, stderr) = analyze_json(&cache, &[wav.to_str().unwrap(), "--no-grid"]);
    assert!(ok, "{stderr}");
    assert_eq!(doc["schema"], 2);
    assert_eq!(doc["cache"], "written");
    let r = &doc["record"];
    assert_eq!(r["schema"], 1);
    assert_eq!(r["spec"]["sampleRate"], 44_100);
    assert_eq!(r["spec"]["channels"], 2);
    assert_eq!(r["frames"], 220_500);
    assert!((r["duration"].as_f64().unwrap() - 5.0).abs() < 1e-9);
    assert_eq!(r["delay"], 0);
    let integrated = r["loudness"]["integrated"].as_f64().expect("integrated");
    assert!((integrated + 20.0).abs() < 0.1, "integrated {integrated}");
    let peak = r["loudness"]["samplePeak"].as_f64().expect("sample peak");
    assert!((peak + 20.0).abs() < 0.05, "sample peak {peak}");
    assert_eq!(r["loudness"]["timeline"]["hopMs"], 100);
    assert_eq!(
        r["loudness"]["timeline"]["shortTerm"]
            .as_array()
            .unwrap()
            .len(),
        50
    );
    assert_eq!(r["grid"], serde_json::Value::Null);
    assert_eq!(r["gridSkipped"], "not requested");
    assert!(r["path"].as_str().unwrap().ends_with("tone.wav"));
    assert!(cache.join(".").exists());
    assert_eq!(
        std::fs::read_dir(&cache)
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1,
        "one cache entry"
    );
}

#[test]
fn second_run_is_a_cache_hit_with_the_identical_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let wav = write_tone_wav(dir.path());
    let (_, first, _) = analyze_json(&cache, &[wav.to_str().unwrap(), "--no-grid"]);
    let (ok, second, stderr) = analyze_json(&cache, &[wav.to_str().unwrap(), "--no-grid"]);
    assert!(ok, "{stderr}");
    assert_eq!(second["cache"], "hit");
    assert_eq!(second["record"], first["record"]);

    // Different settings are a different key: analysed again.
    let (_, other, _) = analyze_json(&cache, &[wav.to_str().unwrap()]);
    assert_eq!(other["cache"], "written");
    assert_eq!(
        other["record"]["gridSkipped"],
        "beat tracking is not available in this build"
    );

    // A touched file is a different key too.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&wav)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let (_, touched, _) = analyze_json(&cache, &[wav.to_str().unwrap(), "--no-grid"]);
    assert_eq!(touched["cache"], "written");
}

#[test]
fn no_cache_bypasses_reads_and_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let wav = write_tone_wav(dir.path());
    let (ok, doc, _) = analyze_json(&cache, &[wav.to_str().unwrap(), "--no-grid", "--no-cache"]);
    assert!(ok);
    assert_eq!(doc["cache"], "bypassed");
    assert!(!cache.exists(), "nothing written");
}

#[test]
fn mp3_fixture_reports_the_lame_trim() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (ok, doc, stderr) = analyze_json(
        dir.path(),
        &[
            fixture("lame-2s.mp3").to_str().unwrap(),
            "--no-grid",
            "--no-cache",
        ],
    );
    assert!(ok, "{stderr}");
    assert_eq!(doc["record"]["frames"], 88_200);
    assert_eq!(doc["record"]["delay"], 1_105);
    assert!(doc["record"]["padding"].as_u64().unwrap() > 0);
}

#[test]
fn text_output_names_the_statistics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wav = write_tone_wav(dir.path());
    sc_cli(dir.path())
        .args(["analyze", wav.to_str().unwrap(), "--no-grid"])
        .assert()
        .success()
        .stdout(predicates::str::contains("loudness  I -20.0 LUFS"))
        .stdout(predicates::str::contains("grid      not requested"))
        .stdout(predicates::str::contains("cache     written"));
}

#[test]
fn unsupported_and_corrupt_files_exit_2_with_error_documents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let text = dir.path().join("notes.wav");
    std::fs::write(&text, "this is not audio\n".repeat(64)).unwrap();
    let flac = std::fs::read(fixture("sine-2s.flac")).unwrap();
    let cut = dir.path().join("cut.flac");
    std::fs::write(&cut, &flac[..flac.len() * 6 / 10]).unwrap();
    let wav = write_tone_wav(dir.path());

    let output = sc_cli(dir.path())
        .args([
            "analyze",
            text.to_str().unwrap(),
            cut.to_str().unwrap(),
            wav.to_str().unwrap(),
            "--no-grid",
            "--no-cache",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let docs: Vec<serde_json::Value> = serde_json::Deserializer::from_str(&stdout)
        .into_iter()
        .map(|d| d.expect("json document"))
        .collect();
    assert_eq!(docs.len(), 3, "{stdout}");
    assert_eq!(docs[0]["error"]["kind"], "unsupportedFormat");
    assert_eq!(docs[1]["error"]["kind"], "corrupt");
    assert!(
        docs[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("truncated")
    );
    assert_eq!(
        docs[2]["cache"], "bypassed",
        "the good file is still analysed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("notes.wav") && stderr.contains("cut.flac"),
        "{stderr}"
    );
}

#[test]
fn cache_path_and_clear() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let wav = write_tone_wav(dir.path());
    analyze_json(&cache, &[wav.to_str().unwrap(), "--no-grid"]);
    sc_cli(&cache)
        .args(["cache", "path"])
        .assert()
        .success()
        .stdout(predicates::str::contains(cache.to_str().unwrap()));
    sc_cli(&cache)
        .args(["cache", "clear"])
        .assert()
        .success()
        .stdout(predicates::str::contains("removed 1 cached"));
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
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
    let dir = tempfile::tempdir().expect("tempdir");
    sc_cli(dir.path())
        .args(["analyze", "/nonexistent.wav", "--no-cache"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("nonexistent.wav"));
}
