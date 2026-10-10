//! `sc-cli apply` and `sc-cli undo` end to end in temp folders, with the backup root in
//! `SC_BACKUP_ROOT`: in place (backup, sidecar, audio changed) then undone byte for byte; a copy
//! with `--out`; neutral tags into WAV, AIFF and FLAC; the JSON document; refusals as three
//! lines and exit code 2. Files are named relative to the folder the command runs in, so lines
//! start with the path as given. The refusals the CLI cannot stage (a rekordbox USB export, a
//! full disk) are covered by the simulated volumes of `sc-io`'s transaction tests; inputs
//! checked together (listed twice, same output name) by `tests/inputs.rs`.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Library, aiff_with_id3, arg, files_under, wav_at, wav_bytes};

const FLAC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/audio/sine-2s.flac"
);
const MP3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/audio/lame-2s.mp3"
);

fn sidecar(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".soundcheck.json");
    name.into()
}

#[test]
fn in_place_backs_up_changes_the_audio_and_undo_restores_every_byte() {
    let lib = Library::new();
    let original = wav_bytes(false);
    let wav = lib.file("Track.wav", &original);
    let run = lib.run_in(&lib.music, &["apply", "Track.wav", "--gain-db", "-3.2"]);
    assert!(run.ok, "{}", run.text());
    let line = run.stdout.trim();
    assert!(
        line.starts_with(
            "Track.wav: gain -3.20 dB, 16-bit dithered, 0 frames trimmed; 0 blocks carried; \
             verified; backup "
        ),
        "{line}"
    );
    assert_eq!(run.stdout.lines().count(), 1, "one line per file");
    let backups = files_under(&lib.backups);
    let backup = backups
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == "Track.wav"))
        .expect("a backup of Track.wav");
    assert!(line.ends_with(&backup.display().to_string()), "{line}");
    assert_eq!(std::fs::read(backup).expect("backup"), original);
    let changed = std::fs::read(&wav).expect("output");
    assert_ne!(changed, original);
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(sidecar(&wav)).expect("sidecar")).expect("JSON");
    assert!(doc.is_object());

    // A second change, then two undos walk back one at a time.
    assert!(lib.run(&["apply", arg(&wav), "--gain-db", "-1"]).ok);
    let run = lib.run(&["undo", arg(&wav)]);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.trim().ends_with(
            "; verified; sidecar describes the previous change again; 1 earlier change remains \
             (undo again to go further back)"
        ),
        "{}",
        run.text()
    );
    assert_eq!(std::fs::read(&wav).expect("first output"), changed);
    let run = lib.run(&["undo", arg(&wav)]);
    assert!(run.ok, "{}", run.text());
    assert_eq!(
        run.stdout.trim(),
        format!(
            "{}: previous version restored from {}; verified; sidecar removed",
            wav.display(),
            backup.display()
        )
    );
    assert_eq!(std::fs::read(&wav).expect("restored"), original);
    assert!(!sidecar(&wav).exists());
    assert!(backup.exists(), "the backup is kept");

    let run = lib.run_in(&lib.music, &["undo", "Track.wav"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert_eq!(
        run.stderr,
        "Track.wav: not undone\n  why: no change of it is recorded in the backup folder\n  \
         what to do: list the recorded changes with sc-cli journal\n"
    );
}

#[test]
fn out_writes_a_copy_and_leaves_the_file() {
    let lib = Library::new();
    let original = wav_bytes(false);
    let wav = lib.file("Track.wav", &original);
    let out = lib.base.join("out");
    let args = [
        "apply",
        "Track.wav",
        "--gain-db",
        "-1",
        "--bits",
        "24",
        "--trim-samples",
        "441",
        "--no-sidecar",
        "--out",
        arg(&out),
    ];
    let run = lib.run_in(&lib.music, &args);
    assert!(run.ok, "{}", run.text());
    let copy = out.join("Track.wav");
    assert_eq!(
        run.stdout.trim(),
        format!(
            "Track.wav: gain -1.00 dB, 24-bit, 441 frames trimmed (441 requested); 0 blocks \
             carried; verified; written to {}",
            copy.display()
        )
    );
    assert_eq!(std::fs::read(&wav).expect("original"), original);
    assert!(copy.is_file() && !sidecar(&copy).exists());
    assert!(
        files_under(&lib.backups)
            .iter()
            .all(|p| p.extension().is_none_or(|e| e != "wav"))
    );

    // An existing file is never replaced.
    let run = lib.run_in(&lib.music, &args);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr.starts_with("Track.wav: not written\n  why: "),
        "{}",
        run.text()
    );
    assert!(run.stderr.contains("already exists"), "{}", run.text());
}

#[test]
fn neutral_tags_go_into_wav_aiff_and_flac_in_one_run() {
    let lib = Library::new();
    let wav = lib.file("Tagged.wav", &wav_bytes(true));
    let aiff = lib.file("Tagged.aiff", &aiff_with_id3());
    lib.file("Plain.wav", &wav_bytes(false));
    let flac = lib.music.join("Sine.flac");
    std::fs::copy(FLAC, &flac).expect("copy flac");
    let files = ["Tagged.wav", "Tagged.aiff", "Plain.wav", "Sine.flac"];
    let mut args = vec!["apply"];
    args.extend(files);
    args.extend([
        "--gain-db",
        "0",
        "--tag",
        "BPM=127.98",
        "--tag",
        "initialkey=8A",
    ]);
    let run = lib.run_in(&lib.music, &args);
    assert_eq!(run.code, Some(0), "{}", run.text());
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", run.text());
    // BPM gives two ID3 frames (TBPM 128 and TXXX:BPM 127.98), INITIALKEY one (TKEY).
    for (line, name) in lines.iter().zip(["Tagged.wav", "Tagged.aiff"]) {
        assert!(
            line.starts_with(&format!(
                "{name}: gain +0.00 dB, 16-bit, 0 frames trimmed; 0 blocks carried, 3 tags \
                 added; verified"
            )),
            "{}",
            run.text()
        );
    }
    assert!(
        lines[2].contains("; tags not added: the file has no ID3 tag; verified"),
        "{}",
        run.text()
    );
    // FLAC: the fields BPM and INITIALKEY.
    assert!(
        lines[3].starts_with("Sine.flac: gain +0.00 dB, 16-bit, 0 frames trimmed;")
            && lines[3].contains(", 2 tags added; verified"),
        "{}",
        run.text()
    );
    for path in [&wav, &aiff] {
        let bytes = std::fs::read(path).expect("tagged");
        for needle in [&b"TBPM"[..], b"128", b"TXXX", b"127.98", b"TKEY", b"8A"] {
            assert!(
                bytes.windows(needle.len()).any(|w| w == needle),
                "{}: {needle:?}",
                path.display()
            );
        }
    }
    let bytes = std::fs::read(&flac).expect("flac");
    for needle in [&b"BPM=127.98"[..], b"INITIALKEY=8A"] {
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "{needle:?}"
        );
    }
}

#[test]
fn frame_ids_and_container_labels_stop_the_run_and_other_names_pass() {
    let lib = Library::new();
    let wav = lib.file("Track.wav", &wav_bytes(true));
    for (tag, why) in [
        ("TBPM=128", "is an ID3 frame id"),
        ("TXXX:BPM=128.00", "use the neutral name (BPM for TXXX:BPM)"),
        ("BPM=fast", "positive number"),
    ] {
        let run = lib.run(&["apply", arg(&wav), "--gain-db", "-1", "--tag", tag]);
        assert_eq!(run.code, Some(2), "{}", run.text());
        assert!(run.stderr.contains(why), "{tag}: {}", run.text());
        assert_eq!(run.stdout, "");
    }
    assert_eq!(std::fs::read(&wav).expect("untouched"), wav_bytes(true));
    assert!(
        !lib.backups.exists(),
        "nothing written, not even the journal"
    );

    // Four-letter names that are not frame ids are ordinary neutral names.
    let args = [
        "apply",
        arg(&wav),
        "--gain-db",
        "0",
        "--tag",
        "MOOD=happy",
        "--tag",
        "YEAR=2026",
    ];
    let run = lib.run(&args);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.contains(", 2 tags added; verified"),
        "{}",
        run.text()
    );
    let bytes = std::fs::read(&wav).expect("tagged");
    for needle in [&b"TXXX"[..], b"MOOD", b"happy", b"YEAR", b"2026"] {
        assert!(
            bytes.windows(needle.len()).any(|w| w == needle),
            "{needle:?}"
        );
    }
}

#[test]
fn json_has_one_document_per_file_with_the_report() {
    let lib = Library::new();
    let a = lib.file("A.wav", &wav_bytes(false));
    let mp3 = lib.music.join("B.mp3");
    std::fs::copy(MP3, &mp3).expect("copy mp3");
    let run = lib.run(&["apply", arg(&mp3), arg(&a), "--gain-db", "-2", "--json"]);
    assert_eq!(run.code, Some(2), "one file failed: {}", run.text());
    let docs = run.docs();
    assert_eq!(docs.len(), 2, "{}", run.text());
    let failed = &docs[0];
    assert_eq!(failed["schema"], 1);
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["kind"], "unsupportedFormat");
    // An MP3 starts with an ID3v2 tag, as a FLAC file may: the message names what is written.
    assert!(
        failed["why"].as_str().is_some_and(|t| t
            .ends_with("only WAV, RF64, AIFF, AIFF-C and FLAC files are written")),
        "{failed}"
    );
    assert!(
        failed["whatToDo"]
            .as_str()
            .is_some_and(|t| t.contains("WAV, AIFF and FLAC"))
    );
    // The failure did not stop the next file.
    let ok = &docs[1];
    assert_eq!(ok["schema"], 1);
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["kind"], "inPlace");
    assert_eq!(ok["file"], arg(&a));
    assert_eq!(ok["request"]["gainDb"], -2.0);
    assert_eq!(ok["request"]["tags"], serde_json::json!([]));
    assert_eq!(ok["render"]["framesIn"], 22_050);
    assert_eq!(ok["render"]["framesOut"], 22_050);
    assert_eq!(ok["render"]["trimFrames"], 0);
    assert_eq!(ok["render"]["trimRequestedFrames"], 0);
    assert_eq!(ok["render"]["bitsOut"], 16);
    assert_eq!(ok["render"]["dithered"], true);
    assert_eq!(ok["outputBlake3"].as_str().map(str::len), Some(64));
    assert!(ok["backup"].as_str().is_some_and(|b| b.contains("A.wav")));
    let steps: Vec<&str> = ok["timings"]
        .as_array()
        .expect("timings")
        .iter()
        .map(|t| t["step"].as_str().expect("step"))
        .collect();
    assert_eq!(
        steps,
        vec![
            "planned",
            "tempWritten",
            "verified",
            "backedUp",
            "renamed",
            "metadataDone",
            "done"
        ]
    );
    assert!(ok["totalMs"].as_f64().is_some_and(|t| t > 0.0));
}

#[test]
fn refusals_say_what_why_and_what_to_do() {
    let lib = Library::new();
    let wav = lib.file("Track.wav", &wav_bytes(false));
    std::os::unix::fs::symlink(&wav, lib.music.join("Link.wav")).expect("symlink");
    let run = lib.run_in(&lib.music, &["apply", "Link.wav", "--gain-db", "-1"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert_eq!(
        run.stderr.lines().collect::<Vec<_>>(),
        vec![
            "Link.wav: not changed",
            "  why: it is a symbolic link",
            "  what to do: name the file it points to instead, or write a copy with --out \
             <folder>",
        ],
        "{}",
        run.text()
    );
    assert_eq!(run.stdout, "");

    let run = lib.run(&["apply", arg(&wav), "--gain-db", "20"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr
            .contains("  why: a gain of +20.00 dB would take the peak to +7."),
        "{}",
        run.text()
    );
    assert!(
        run.stderr.contains("  what to do: use --gain-db +12."),
        "{}",
        run.text()
    );

    let hi_res = lib.file("Hi-res.wav", &wav_at(96_000, false));
    let run = lib.run(&["apply", arg(&hi_res), "--gain-db", "-1"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr.contains("sample rate 96000 Hz"),
        "{}",
        run.text()
    );
    assert!(
        run.stderr
            .contains("  what to do: convert it to 44.1 or 48 kHz in an audio editor first"),
        "{}",
        run.text()
    );

    let before = std::fs::read(&wav).expect("read");
    std::fs::set_permissions(&wav, std::fs::Permissions::from_mode(0o444)).expect("chmod");
    let privileged = std::fs::OpenOptions::new().write(true).open(&wav).is_ok();
    if !privileged {
        let run = lib.run(&["apply", arg(&wav), "--gain-db", "-1"]);
        assert_eq!(run.code, Some(2), "{}", run.text());
        assert!(
            run.stderr.contains("  why: it is read-only\n"),
            "{}",
            run.text()
        );
    }
    assert_eq!(std::fs::read(&wav).expect("read"), before, "untouched");
    let leftovers: Vec<_> = std::fs::read_dir(&lib.music)
        .expect("list")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "no temp file left: {leftovers:?}");
}

/// The ramp in the test WAV crosses zero at frame 500 (left 0, right 8), so a cut requested at
/// 520 is made 20 frames earlier, at the quietest frame within 1 ms; text and JSON say both.
#[test]
fn apply_reports_the_cut_made_and_the_one_requested() {
    let lib = Library::new();
    let a = lib.file("A.wav", &wav_bytes(false));
    let out = lib.base.join("out");
    let text = lib.run(&[
        "apply",
        arg(&a),
        "--gain-db",
        "0",
        "--trim-samples",
        "520",
        "--out",
        arg(&out),
    ]);
    assert!(text.ok, "{}", text.text());
    assert!(
        text.stdout
            .contains(", 500 frames trimmed (520 requested);"),
        "{}",
        text.stdout
    );
    let json_out = lib.base.join("json");
    let run = lib.run(&[
        "apply",
        arg(&a),
        "--gain-db",
        "0",
        "--trim-samples",
        "520",
        "--out",
        arg(&json_out),
        "--json",
    ]);
    assert!(run.ok, "{}", run.text());
    let doc = &run.docs()[0];
    assert_eq!(doc["request"]["trimFrames"], 520);
    assert_eq!(doc["render"]["trimFrames"], 500);
    assert_eq!(doc["render"]["trimRequestedFrames"], 520);
    assert_eq!(doc["render"]["framesOut"], 22_050 - 500);
    assert_eq!(doc["render"]["exact"], false, "faded in");
}
