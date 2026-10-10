//! `sc-cli process` end to end on synthetic WAVs: in place with a backup, a copy with `--out`,
//! JSON, and refusals. Loudness only (`--no-grid`), so no model is needed; the backup root,
//! cache and edits live in a temp folder.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use common::{Library, Run, arg};
use sc_core::{AudioSpec, testsig};

/// A 5 s stereo 1 kHz tone at -20 dBFS (S-P95 -20.0 LUFS, true peak -20 dBTP) at `rate`.
fn tone(lib: &Library, name: &str, rate: u32) -> PathBuf {
    let path = lib.music.join(name);
    let buf = testsig::sine(AudioSpec::new(rate, 2), 1000.0, 0.1, 5.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
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

fn process(lib: &Library, args: &[&str]) -> Run {
    let output = Command::cargo_bin("sc-cli")
        .expect("binary")
        .env("SC_BACKUP_ROOT", &lib.backups)
        .env("SC_CACHE_DIR", lib.base.join("cache"))
        .env("SC_EDITS_DIR", lib.base.join("edits"))
        .current_dir(&lib.base)
        .arg("process")
        .args(args)
        .arg("--no-grid")
        .output()
        .expect("run");
    Run::from(output)
}

fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read")
}

fn frames(path: &Path) -> u32 {
    hound::WavReader::open(path).expect("wav").duration()
}

#[test]
fn in_place_writes_backs_up_and_records_the_export() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav", 44_100);
    let original = bytes(&wav);
    let run = process(&lib, &[arg(&wav), "--batch-mode", "library"]);
    assert!(run.ok, "{}", run.text());
    let first = run.stdout.lines().next().expect("a line");
    assert!(
        first.starts_with(&format!(
            "{}: library: Gain +9.0 dB, length kept; tags REPLAYGAIN_TRACK_GAIN, \
             REPLAYGAIN_TRACK_PEAK, SOUNDCHECK; written and verified",
            wav.display()
        )),
        "{}",
        run.text()
    );
    // The fixture has no tag to write into, and the line says so.
    assert!(first.contains("; tags not added: "), "{first}");
    assert!(first.contains("; backup "), "{first}");
    assert!(
        run.stdout
            .contains("1 files: 1 written, 0 XML only, 0 skipped, 0 failed"),
        "{}",
        run.text()
    );
    assert_ne!(bytes(&wav), original);
    assert_eq!(frames(&wav), 220_500);
    let sidecar: serde_json::Value = serde_json::from_slice(&bytes(&PathBuf::from(format!(
        "{}.soundcheck.json",
        wav.display()
    ))))
    .expect("sidecar");
    assert_eq!(sidecar["schema"], 2);
    assert_eq!(sidecar["export"]["settings"]["batchMode"], "library");
    assert_eq!(sidecar["export"]["plan"]["expectFrames"], 220_500);
    // The original is in the backup root.
    let backed_up = common::files_under(&lib.backups)
        .into_iter()
        .any(|p| p.file_name().is_some_and(|n| n == "a.wav") && bytes(&p) == original);
    assert!(backed_up);
}

#[test]
fn out_writes_a_copy_and_leaves_the_file() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav", 44_100);
    let original = bytes(&wav);
    let out = lib.base.join("exported");
    let run = process(
        &lib,
        &[
            arg(&wav),
            "--mode",
            "prepare",
            "--out",
            arg(&out),
            "--depth",
            "24",
        ],
    );
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.contains(
            "a.wav: prepare: Gain +9.0 dB, Not cut: no grid, 24-bit; tags REPLAYGAIN_TRACK_GAIN, \
             REPLAYGAIN_TRACK_PEAK, SOUNDCHECK; written and verified"
        ),
        "{}",
        run.text()
    );
    assert!(
        run.stdout
            .contains(&format!("; written to {}", out.join("a.wav").display())),
        "{}",
        run.text()
    );
    assert_eq!(bytes(&wav), original);
    let copy = out.join("a.wav");
    assert_eq!(frames(&copy), frames(&wav));
    assert_eq!(
        hound::WavReader::open(&copy)
            .expect("wav")
            .spec()
            .bits_per_sample,
        24
    );
    // A copy needs no backup.
    assert!(
        common::files_under(&lib.backups)
            .iter()
            .all(|p| p.file_name().is_none_or(|n| n != "a.wav"))
    );
}

#[test]
fn json_has_one_document_per_file() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav", 44_100);
    let hi_res = tone(&lib, "b.wav", 96_000);
    let run = process(
        &lib,
        &[arg(&wav), arg(&hi_res), "--batch-mode", "library", "--json"],
    );
    assert!(run.ok, "{}", run.text());
    let docs = run.docs();
    assert_eq!(docs.len(), 2, "{}", run.text());
    assert_eq!(docs[0]["ok"], true);
    assert_eq!(docs[0]["export"]["type"], "write");
    assert_eq!(
        docs[0]["written"]["framesIn"],
        docs[0]["written"]["framesOut"]
    );
    assert_eq!(docs[0]["written"]["outputAnalysed"], true);
    assert_eq!(docs[1]["export"]["type"], "skip");
    assert_eq!(docs[1]["export"]["reason"]["type"], "notDjSafeRate");
    assert!(docs[1].get("written").is_none());
}

#[test]
fn refusals_print_three_lines_and_fail_the_run() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav", 44_100);
    let missing = lib.music.join("missing.wav");
    let run = process(
        &lib,
        &[
            arg(&wav),
            arg(&wav),
            arg(&missing),
            "--batch-mode",
            "library",
        ],
    );
    assert!(!run.ok);
    assert_eq!(run.code, Some(2));
    assert!(
        run.stdout
            .contains("3 files: 1 written, 0 XML only, 0 skipped, 2 failed"),
        "{}",
        run.text()
    );
    assert!(run.stderr.contains("listed twice"), "{}", run.text());
    let refusal = run
        .stderr
        .lines()
        .skip_while(|l| !l.contains("listed twice"))
        .take(3)
        .count();
    assert!(refusal >= 2, "{}", run.text());

    // A skipped file is not a failure.
    let lib = Library::new();
    let hi_res = tone(&lib, "b.wav", 96_000);
    let run = process(&lib, &[arg(&hi_res), "--batch-mode", "prepare"]);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.contains("b.wav: prepare: skipped"),
        "{}",
        run.text()
    );
    assert!(
        run.stdout.contains("  why: its sample rate, 96000 Hz"),
        "{}",
        run.text()
    );
}
