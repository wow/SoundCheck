//! The batch artefacts end to end: `sc-cli process` writes `soundcheck-rekordbox.xml` and
//! `grid-report.csv` (into `--out`, or a new folder under `SC_EXPORTS_ROOT` in place; neither
//! with `--no-xml`), and `sc-cli xml` writes the XML of files named, reading an export's grid
//! back from its sidecar. Loudness only (`--no-grid`) except the click-track test, which needs
//! the model files and is skipped without them. Everything lives in a temp folder.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use common::{Library, Run, arg};
use sc_core::{AudioSpec, testsig};

/// A 5 s stereo 1 kHz tone at -20 dBFS, 44.1 kHz 16-bit.
fn tone(lib: &Library, name: &str) -> PathBuf {
    let path = lib.music.join(name);
    let buf = testsig::sine(AudioSpec::new(44_100, 2), 1000.0, 0.1, 5.0);
    write_wav(&path, &buf.data, 1.0);
    path
}

fn write_wav(path: &Path, data: &[f32], scale: f64) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for s in data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * scale * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
}

/// The workspace's model directory, when the models are installed.
fn models() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let ok =
        dir.join("mel_spectrogram.onnx").is_file() && dir.join("beat_this_small.onnx").is_file();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok.then_some(dir)
}

fn sc_cli(lib: &Library, args: &[&str]) -> Run {
    let mut cmd = Command::cargo_bin("sc-cli").expect("binary");
    cmd.env("SC_BACKUP_ROOT", &lib.backups)
        .env("SC_CACHE_DIR", lib.base.join("cache"))
        .env("SC_EDITS_DIR", lib.base.join("edits"))
        .env("SC_EXPORTS_ROOT", lib.base.join("exports"))
        .current_dir(&lib.base)
        .args(args);
    if let Some(m) = models() {
        cmd.env("SC_MODEL_DIR", m);
    }
    Run::from(cmd.output().expect("run"))
}

fn read(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).expect("read")).expect("UTF-8")
}

/// The batch folders under the exports root.
fn batch_folders(lib: &Library) -> Vec<PathBuf> {
    let Ok(list) = std::fs::read_dir(lib.base.join("exports")) else {
        return Vec::new();
    };
    list.map(|e| e.expect("entry").path()).collect()
}

#[test]
fn process_in_place_writes_the_artefacts_into_a_new_batch_folder() {
    let lib = Library::new();
    let wav = tone(&lib, "Şarkı & co.wav");
    let run = sc_cli(
        &lib,
        &["process", arg(&wav), "--batch-mode", "prepare", "--no-grid"],
    );
    assert!(run.ok, "{}", run.text());
    let folders = batch_folders(&lib);
    assert_eq!(folders.len(), 1, "{folders:?}");
    let xml_path = folders[0].join("soundcheck-rekordbox.xml");
    let csv_path = folders[0].join("grid-report.csv");
    assert!(
        run.stdout.contains(&format!(
            "rekordbox XML: {} (0 tracks with a grid; playlist \"SoundCheck 20",
            xml_path.display()
        )),
        "{}",
        run.text()
    );
    assert!(
        run.stdout
            .contains(&format!("grid report: {}", csv_path.display())),
        "{}",
        run.text()
    );
    let xml = read(&xml_path);
    // Loudness only: no grid, so the file is not listed (importing it could only change its
    // information in rekordbox); the report says why.
    assert!(xml.contains("<COLLECTION Entries=\"0\"/>"), "{xml}");
    // The playlist is named after the batch's folder.
    let folder = folders[0]
        .file_name()
        .and_then(|n| n.to_str())
        .expect("name");
    assert!(
        xml.contains(&format!("Name=\"SoundCheck {folder}\"")),
        "{xml}"
    );
    let csv = read(&csv_path);
    let mut lines = csv.trim_start_matches('\u{FEFF}').lines();
    assert_eq!(
        lines.next(),
        Some("file,mode,action,gain_db,cut_s,bpm,bar1_s,grid,grid_check,notes")
    );
    let row = lines.next().expect("a row");
    assert!(
        row.starts_with(&format!("{},prepare,written,+8.9", wav.display())),
        "{row}"
    );
    // No cut, BPM or bar 1 without a grid; the note says why no tag was written.
    assert!(
        row.ends_with(",,,,not in the XML: no grid,,tags not added: the file has no ID3 tag"),
        "{row}"
    );
    // A second batch gets its own folder.
    let again = sc_cli(
        &lib,
        &["process", arg(&wav), "--batch-mode", "prepare", "--no-grid"],
    );
    assert!(again.ok, "{}", again.text());
    assert_eq!(batch_folders(&lib).len(), 2);
}

#[test]
fn process_out_writes_the_artefacts_next_to_the_copies() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav");
    let hi_res = lib.music.join("b.wav");
    let buf = testsig::sine(AudioSpec::new(96_000, 2), 1000.0, 0.1, 1.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 96_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&hi_res, spec).expect("wav");
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * 32767.0).round() as i16)
            .expect("write");
    }
    w.finalize().expect("finalize");
    let out = lib.base.join("out");
    let run = sc_cli(
        &lib,
        &[
            "process",
            arg(&wav),
            arg(&hi_res),
            "--batch-mode",
            "prepare",
            "--out",
            arg(&out),
            "--no-grid",
            "--json",
        ],
    );
    assert!(run.ok, "{}", run.text());
    // With --json, stdout keeps one document per file; the artefacts are named on stderr.
    assert_eq!(run.docs().len(), 2);
    assert!(run.stderr.contains("rekordbox XML: "), "{}", run.text());
    let xml = read(&out.join("soundcheck-rekordbox.xml"));
    assert!(xml.contains("<COLLECTION Entries=\"0\"/>"), "{xml}");
    let csv = read(&out.join("grid-report.csv"));
    let rows: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(rows.len(), 2, "{csv}");
    assert!(rows[1].contains(",prepare,skipped,"), "{csv}");
    assert!(rows[1].contains("96000 Hz"), "{csv}");
    assert_eq!(batch_folders(&lib).len(), 0);
}

#[test]
fn no_xml_writes_no_artefacts() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav");
    let run = sc_cli(
        &lib,
        &[
            "process",
            arg(&wav),
            "--batch-mode",
            "library",
            "--no-grid",
            "--no-xml",
        ],
    );
    assert!(run.ok, "{}", run.text());
    assert!(!run.stdout.contains("rekordbox XML"), "{}", run.text());
    assert_eq!(batch_folders(&lib).len(), 0);
}

/// Gives the export recorded in `wav`'s sidecar a trusted 4/4 grid at 120 BPM with bar 1 at
/// 221 samples (5 ms), as a Prepare cut leaves it. The audio is untouched, so the sidecar still
/// describes the file.
fn record_grid(wav: &Path) {
    let path = PathBuf::from(format!("{}.soundcheck.json", wav.display()));
    let mut doc: serde_json::Value = serde_json::from_str(&read(&path)).expect("sidecar");
    doc["export"]["grid"] = serde_json::json!({
        "bar1": 221,
        "firstBarLine": 221,
        "bpm": 120.0,
        "bpmExact": 120.004,
        "meter": {"beatsPerBar": 4, "unit": "quarter", "grouping": [1, 1, 1, 1]},
        "edited": false,
        "confirmed": true
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).expect("json")).expect("write");
}

/// Appends an ID3v2.3 `id3 ` chunk with a `TIT2` title and a `TPE1` artist (UTF-16 with a
/// byte-order mark) to the WAV at `path`.
fn tag_wav(path: &Path, title: &str, artist: &str) {
    let frame = |id: &[u8; 4], text: &str| {
        let mut body = vec![1_u8, 0xFF, 0xFE];
        body.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let mut f = id.to_vec();
        f.extend_from_slice(&u32::try_from(body.len()).expect("small").to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend(body);
        f
    };
    let mut frames = frame(b"TIT2", title);
    frames.extend(frame(b"TPE1", artist));
    let size = u32::try_from(frames.len()).expect("small");
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    tag.extend([21, 14, 7, 0].map(|shift| u8::try_from((size >> shift) & 0x7F).expect("7 bits")));
    tag.extend(frames);
    let mut bytes = std::fs::read(path).expect("read");
    bytes.extend_from_slice(b"id3 ");
    bytes.extend_from_slice(&u32::try_from(tag.len()).expect("small").to_le_bytes());
    bytes.extend(&tag);
    if tag.len() % 2 == 1 {
        bytes.push(0);
    }
    let riff = u32::try_from(bytes.len() - 8).expect("small");
    bytes[4..8].copy_from_slice(&riff.to_le_bytes());
    std::fs::write(path, bytes).expect("write");
}

#[test]
fn xml_reads_an_export_back_from_its_sidecar() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav");
    tag_wav(&wav, "Gece Yarısı", "Ayşe & İlhan");
    let run = sc_cli(
        &lib,
        &["process", arg(&wav), "--batch-mode", "library", "--no-grid"],
    );
    assert!(run.ok, "{}", run.text());
    record_grid(&wav);
    let changed = tone(&lib, "b.wav");
    let out = lib.base.join("my.xml");
    let run = sc_cli(
        &lib,
        &[
            "xml",
            arg(&wav),
            arg(&changed),
            "--out",
            arg(&out),
            "--no-grid",
        ],
    );
    assert!(run.ok, "{}", run.text());
    let xml = read(&out);
    // Named by its own tags; only the attributes SoundCheck sets.
    let location = sc_io::rekordbox::location(&wav);
    assert!(
        xml.contains(&format!(
            "<TRACK Name=\"Gece Yarısı\" Artist=\"Ayşe &amp; İlhan\" TotalTime=\"5\" \
             AverageBpm=\"120.00\" Location=\"{location}\">\n      \
             <TEMPO Inizio=\"0.005\" Bpm=\"120.00\" Metro=\"4/4\" Battito=\"1\"/>"
        )),
        "{xml}"
    );
    // The changed file has no grid: not listed.
    assert!(xml.contains("<COLLECTION Entries=\"1\">"), "{xml}");
    assert!(
        run.stdout.contains(&format!(
            "{}: exported earlier; tempo: beat 1 at 0.005 s; 120.00 BPM",
            wav.display()
        )),
        "{}",
        run.text()
    );
    assert!(
        run.stdout.contains(&format!(
            "{}: analysed; not in the XML: no grid",
            changed.display()
        )),
        "{}",
        run.text()
    );
    assert!(
        run.stdout
            .contains("(1 tracks, 1 not listed (no grid the XML may carry), 0 failed;"),
        "{}",
        run.text()
    );
    // Once the file changes, its sidecar no longer describes it: it is analysed again.
    let mut bytes = std::fs::read(&wav).expect("read");
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&wav, bytes).expect("write");
    let run = sc_cli(&lib, &["xml", arg(&wav), "--no-grid"]);
    assert!(run.ok, "{}", run.text());
    // The XML goes to stdout, the lines to stderr.
    assert!(run.stdout.starts_with("<?xml"), "{}", run.text());
    assert!(!run.stdout.contains("<TEMPO"), "{}", run.text());
    assert!(
        run.stderr
            .contains("analysed; not in the XML: no grid; changed since its export"),
        "{}",
        run.text()
    );
}

#[test]
fn xml_out_never_replaces_a_file_soundcheck_did_not_write() {
    let lib = Library::new();
    let a = tone(&lib, "a.wav");
    let b = tone(&lib, "b.wav");
    let a_bytes = std::fs::read(&a).expect("read");
    // `--out *.wav` as a shell expands it: `--out a.wav b.wav`.
    let run = sc_cli(&lib, &["xml", "--no-grid", "--out", arg(&a), arg(&b)]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert_eq!(
        std::fs::read(&a).expect("read"),
        a_bytes,
        "the audio is untouched"
    );
    let lines: Vec<&str> = run.stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{}", run.text());
    assert_eq!(lines[0], format!("{}: not written", a.display()));
    assert!(
        lines[1].starts_with("  why: ") && lines[1].contains(".xml"),
        "{}",
        run.text()
    );
    assert!(lines[2].starts_with("  what to do: "), "{}", run.text());
    // An existing file ending in .xml that SoundCheck did not write is not replaced either.
    let theirs = lib.base.join("rekordbox.xml");
    let rekordbox = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n  <PRODUCT Name=\"rekordbox\"/>\n</DJ_PLAYLISTS>\n";
    std::fs::write(&theirs, rekordbox).expect("write");
    let run = sc_cli(&lib, &["xml", "--no-grid", "--out", arg(&theirs), arg(&b)]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stderr.contains("SoundCheck did not write"),
        "{}",
        run.text()
    );
    assert_eq!(read(&theirs), rekordbox);
    // Its own XML is replaced.
    let ours = lib.base.join("ours.XML");
    for _ in 0..2 {
        let run = sc_cli(&lib, &["xml", "--no-grid", "--out", arg(&ours), arg(&b)]);
        assert!(run.ok, "{}", run.text());
    }
}

#[test]
fn xml_fails_for_a_missing_file_and_lists_the_rest() {
    let lib = Library::new();
    let wav = tone(&lib, "a.wav");
    let missing = lib.music.join("missing.wav");
    let run = sc_cli(&lib, &["xml", arg(&missing), arg(&wav), "--no-grid"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(run.stdout.starts_with("<?xml"), "{}", run.text());
    assert!(
        run.stderr
            .contains("0 tracks, 1 not listed (no grid the XML may carry), 1 failed"),
        "{}",
        run.text()
    );
}

#[test]
fn prepare_xml_puts_bar_1_at_the_lead() {
    let Some(_) = models() else { return };
    let lib = Library::new();
    // 0.3 s of silence, then 64 clicks at 120 BPM, the first of every four louder.
    let path = lib.music.join("click.wav");
    let clicks = testsig::click_track(AudioSpec::CD, sc_core::Bpm(120.0), 64, 15.0);
    let mut data = vec![0.0_f32; 2 * testsig::frames_for(AudioSpec::CD, 0.3)];
    data.extend_from_slice(&clicks.data);
    write_wav(&path, &data, 0.5);
    let out = lib.base.join("out");
    let run = sc_cli(
        &lib,
        &[
            "process",
            arg(&path),
            "--batch-mode",
            "prepare",
            "--out",
            arg(&out),
        ],
    );
    assert!(run.ok, "{}", run.text());
    assert!(run.stdout.contains(", Cut 0."), "{}", run.text());
    let xml = read(&out.join("soundcheck-rekordbox.xml"));
    let ok = [
        r#"<TEMPO Inizio="0.005" Bpm="120.00" Metro="4/4" Battito="1"/>"#,
        r#"<TEMPO Inizio="0.006" Bpm="120.00" Metro="4/4" Battito="1"/>"#,
    ];
    assert!(ok.iter().any(|t| xml.contains(t)), "{xml}");
    // The copy is listed, named after the file (it has no title tag).
    let copy = sc_io::rekordbox::location(&out.join("click.wav"));
    assert!(
        xml.contains(&format!(
            "<TRACK Name=\"click\" TotalTime=\"32\" AverageBpm=\"120.00\" Location=\"{copy}\">"
        )),
        "{xml}"
    );
    let csv = read(&out.join("grid-report.csv"));
    assert!(csv.contains(",120.00,0.00"), "{csv}");
}
