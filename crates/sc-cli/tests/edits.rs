//! Grid edits saved by the app in `sc-cli plan` and `sc-cli labels`: a confirmed grid needs no
//! review and is printed as a label. Needs the model files (skipped without); the cache and the
//! edits live in a temp directory.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use sc_core::analysis::{AnalysisSettings, GridEdit, Model};
use sc_core::{AudioSpec, Bpm, testsig};
use sc_io::cache::Cache;
use sc_io::edits::{EDIT_SCHEMA, EditStore, SavedEdit};

fn models() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let ok =
        dir.join("mel_spectrogram.onnx").is_file() && dir.join("beat_this_small.onnx").is_file();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok.then_some(dir)
}

/// 0.4 s of silence, then 72 clicks at 124 BPM, 16-bit stereo 44.1 kHz.
fn click_wav(dir: &Path) -> PathBuf {
    let path = dir.join("click, 124.wav");
    let clicks = testsig::click_track(AudioSpec::CD, Bpm(124.0), 72, 15.0);
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

fn sc_cli(dir: &Path, models: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("sc-cli")
        .expect("binary")
        .env("SC_CACHE_DIR", dir.join("cache"))
        .env("SC_EDITS_DIR", dir.join("edits"))
        .env("SC_MODEL_DIR", models)
        .args(args)
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Saves `confirmed` for the file as it is now, made under the 130-180 BPM range.
fn save(dir: &Path, wav: &Path, edit: GridEdit, confirmed: bool) {
    let settings = AnalysisSettings {
        bpm_range: (Bpm(130.0), Bpm(180.0)),
        grid: true,
        model: Model::Small,
    };
    let (path, key) = Cache::key_for(wav, &settings).unwrap();
    EditStore::open(dir.join("edits"))
        .put(&SavedEdit {
            schema: EDIT_SCHEMA,
            path,
            size: key.size,
            mtime_ns: key.mtime_ns,
            bpm_range: settings.bpm_range,
            edit,
            confirmed,
        })
        .unwrap();
}

#[test]
fn a_confirmed_grid_leaves_review_in_plan_and_is_printed_as_a_label() {
    let Some(models) = models() else { return };
    let dir = tempfile::tempdir().unwrap();
    let wav = click_wav(dir.path());
    let file = wav.to_str().unwrap();
    // 124 BPM is outside 130-180, so the row needs review.
    let before = sc_cli(
        dir.path(),
        &models,
        &["plan", file, "--bpm-range", "130-180"],
    );
    assert!(before.contains("1 need review"), "{before}");
    let labels = sc_cli(
        dir.path(),
        &models,
        &["labels", file, "--bpm-range", "130-180"],
    );
    let mut lines = labels.lines();
    assert_eq!(
        lines.next(),
        Some("file,bpm,bar1_s,meter,grouping,confirmed")
    );
    let row = lines.next().unwrap();
    assert!(row.starts_with("\"click, 124.wav\",124.00,"), "{row}");
    assert!(row.ends_with(",4/4,,no"), "{row}");
    let only = sc_cli(
        dir.path(),
        &models,
        &["labels", file, "--bpm-range", "130-180", "--confirmed-only"],
    );
    assert!(
        only.contains("# \"click, 124.wav\": not confirmed"),
        "{only}"
    );

    save(dir.path(), &wav, GridEdit::default(), true);
    let after = sc_cli(
        dir.path(),
        &models,
        &["plan", file, "--bpm-range", "130-180"],
    );
    assert!(after.contains("0 need review"), "{after}");
    assert!(after.contains("(grid confirmed)"), "{after}");
    let json = sc_cli(
        dir.path(),
        &models,
        &["plan", file, "--bpm-range", "130-180", "--json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(doc["gridConfirmed"], true);
    assert_eq!(doc["gridEdited"], false);
    let labels = sc_cli(
        dir.path(),
        &models,
        &["labels", file, "--bpm-range", "130-180", "--confirmed-only"],
    );
    assert!(labels.lines().nth(1).unwrap().ends_with(",yes"), "{labels}");

    // An edit moving beat 1 to beat 2 shows up in both.
    save(
        dir.path(),
        &wav,
        GridEdit {
            downbeat_shift: 1,
            ..GridEdit::default()
        },
        true,
    );
    let edited = sc_cli(
        dir.path(),
        &models,
        &["plan", file, "--bpm-range", "130-180"],
    );
    assert!(edited.contains("(grid edited and confirmed)"), "{edited}");
    let labels = sc_cli(
        dir.path(),
        &models,
        &["labels", file, "--bpm-range", "130-180"],
    );
    let bar1: f64 = labels
        .lines()
        .nth(1)
        .unwrap()
        .split(',')
        .nth(3)
        .unwrap()
        .parse()
        .unwrap();
    let base: f64 = row.split(',').nth(3).unwrap().parse().unwrap();
    assert!(
        (bar1 - base - 60.0 / 124.0).abs() <= 0.002,
        "{bar1} vs {base}"
    );
}
