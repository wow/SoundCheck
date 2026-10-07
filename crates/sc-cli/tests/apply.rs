//! `sc-cli apply` and `sc-cli undo` end to end in temp folders, with the backup root in
//! `SC_BACKUP_ROOT`: in place (backup, sidecar, audio changed) then undone byte for byte; a copy
//! with `--out`; tag edits; the JSON document; refusals as three lines and exit code 2. The
//! refusals the CLI cannot stage (a rekordbox USB export, a full disk) are covered by the
//! simulated volumes of `sc-io`'s transaction tests.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Library, arg, files_under, wav_bytes};

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
    let run = lib.run(&["apply", arg(&wav), "--gain-db", "-3.2"]);
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
    assert_ne!(std::fs::read(&wav).expect("output"), original);
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(sidecar(&wav)).expect("sidecar")).expect("JSON");
    assert!(doc.is_object());

    let run = lib.run(&["undo", arg(&wav)]);
    assert!(run.ok, "{}", run.text());
    assert_eq!(
        run.stdout.trim(),
        format!(
            "Track.wav: original restored from {}; verified; sidecar removed",
            backup.display()
        )
    );
    assert_eq!(std::fs::read(&wav).expect("restored"), original);
    assert!(!sidecar(&wav).exists());
    assert!(backup.exists(), "the backup is kept");

    let run = lib.run(&["undo", arg(&wav)]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr
            .contains("Track.wav: not undone\n  why: no change of it is recorded"),
        "{}",
        run.text()
    );
}

#[test]
fn out_writes_a_copy_and_leaves_the_file() {
    let lib = Library::new();
    let original = wav_bytes(false);
    let wav = lib.file("Track.wav", &original);
    let out = lib.base.join("out");
    let run = lib.run(&[
        "apply",
        arg(&wav),
        "--gain-db",
        "-1",
        "--bits",
        "24",
        "--trim-samples",
        "441",
        "--no-sidecar",
        "--out",
        arg(&out),
    ]);
    assert!(run.ok, "{}", run.text());
    let copy = out.join("Track.wav");
    assert_eq!(
        run.stdout.trim(),
        format!(
            "Track.wav: gain -1.00 dB, 24-bit, 441 frames trimmed; 0 blocks carried; verified; \
             written to {}",
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
    let run = lib.run(&["apply", arg(&wav), "--gain-db", "-1", "--out", arg(&out)]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr.starts_with("Track.wav: not written\n  why: "),
        "{}",
        run.text()
    );
    assert!(run.stderr.contains("already exists"), "{}", run.text());
}

#[test]
fn tags_are_added_to_an_id3_chunk_or_their_absence_is_said() {
    let lib = Library::new();
    let tagged = lib.file("Tagged.wav", &wav_bytes(true));
    let plain = lib.file("Plain.wav", &wav_bytes(false));
    let flac = lib.music.join("Sine.flac");
    std::fs::copy(FLAC, &flac).expect("copy flac");
    let run = lib.run(&[
        "apply",
        arg(&tagged),
        arg(&plain),
        arg(&flac),
        "--gain-db",
        "0",
        "--tag",
        "TBPM=128",
        "--tag",
        "TXXX:BPM=128.00",
    ]);
    // The FLAC field names are not ID3 labels; whatever the FLAC's tags, the WAVs decide.
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert!(
        lines[0].starts_with(
            "Tagged.wav: gain +0.00 dB, 16-bit, 0 frames trimmed; 0 blocks carried, 2 tags added; verified"
        ),
        "{}",
        run.text()
    );
    assert!(
        lines[1].contains("; tags not added: the file has no ID3 tag; verified"),
        "{}",
        run.text()
    );
    assert!(
        run.stdout.contains("Sine.flac: gain +0.00 dB")
            || run.stderr.contains("Sine.flac: not changed"),
        "{}",
        run.text()
    );
    let bytes = std::fs::read(&tagged).expect("tagged");
    assert!(bytes.windows(4).any(|w| w == b"TBPM"));
}

#[test]
fn json_has_one_document_per_file_with_the_report() {
    let lib = Library::new();
    let a = lib.file("A.wav", &wav_bytes(false));
    let mp3 = lib.music.join("B.mp3");
    std::fs::copy(MP3, &mp3).expect("copy mp3");
    let run = lib.run(&["apply", arg(&a), arg(&mp3), "--gain-db", "-2", "--json"]);
    assert_eq!(run.code, Some(2), "one file failed: {}", run.text());
    let docs = run.docs();
    assert_eq!(docs.len(), 2, "{}", run.text());
    let ok = &docs[0];
    assert_eq!(ok["schema"], 1);
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["kind"], "in_place");
    assert_eq!(ok["file"], arg(&a));
    assert_eq!(ok["request"]["gainDb"], -2.0);
    assert_eq!(ok["render"]["framesIn"], 22_050);
    assert_eq!(ok["render"]["framesOut"], 22_050);
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
    assert_eq!(steps.first(), Some(&"planned"));
    assert!(ok["totalMs"].as_f64().is_some_and(|t| t > 0.0));
    let failed = &docs[1];
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["kind"], "unsupportedFormat");
    // An MP3 starts with an ID3v2 tag, as a FLAC file may: the message names what is written.
    assert!(
        failed["why"]
            .as_str()
            .is_some_and(|t| t.ends_with("only WAV, RF64, AIFF, AIFF-C and FLAC files are written")),
        "{failed}"
    );
    assert!(
        failed["whatToDo"]
            .as_str()
            .is_some_and(|t| t.contains("WAV, AIFF and FLAC"))
    );
}

#[test]
fn refusals_say_what_why_and_what_to_do() {
    let lib = Library::new();
    let wav = lib.file("Track.wav", &wav_bytes(false));
    let link = lib.music.join("Link.wav");
    std::os::unix::fs::symlink(&wav, &link).expect("symlink");
    let run = lib.run(&["apply", arg(&link), "--gain-db", "-1"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert_eq!(
        run.stderr.lines().collect::<Vec<_>>(),
        vec![
            "Link.wav: not changed",
            "  why: it is a symbolic link",
            "  what to do: apply to the file it points to, or write a copy with --out <folder>",
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
