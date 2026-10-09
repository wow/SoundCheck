//! Opt-in: the writers on real files, checked by the matrix's independent reader and rules.
//!
//! `SC_REAL_FIXTURES=<dir>` names a folder of WAV, AIFF and FLAC files (sub-folders and other
//! files are skipped; the folder is only read). Each file is rendered into a temp folder through
//! the write transaction, as `sc-cli apply --out <dir> --gain-db -1 --tag BPM=128.00` does
//! (`txn::apply_to_folder`), and the output must pass every check of a matrix output
//! (`check::check_output`): the same blocks in the same order, every chunk, metadata block, tag
//! item and the trailing bytes byte-identical unless the rules in `cases.rs` say otherwise, the
//! BPM edit in the existing tag (or the documented reason why none is made), DJ-safe headers
//! with the source's rate, channels and frame count, and the PCM within the matrix tolerances.
//! Then a temp copy is changed in place (the output must be the same bytes) and undone, and
//! must be byte-identical to the original with no sidecar left. Files the renderer refuses
//! (a float source that would clip, a format that is not DJ-safe) are reported, not failed.
//! One line per file goes to stderr (`cargo test -p sc-io --test matrix real -- --nocapture`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use sc_core::Error;
use sc_io::txn::{self, TxnOptions};

use super::apply::{Applied, ApplyArgs, IDENTITY, TagEdit, render_request};
use super::cases::{BlockExpect, Fixture, Symphonia, rule};
use super::check::check_output;
use super::decode::decode;
use super::expect::{edited_tag, tags_not_added};
use super::inspect::{iff_pcm, streaminfo};
use super::parse::{Container, Kind, Parsed, parse};
use super::pcm::Samples;

/// The environment variable naming the folder.
const ENV: &str = "SC_REAL_FIXTURES";

const ARGS: ApplyArgs = ApplyArgs {
    gain_db: -1.0,
    ..IDENTITY
};

/// The BPM edit as `sc-cli apply --tag BPM=128.00` maps it.
fn bpm_edits(container: Container) -> Vec<TagEdit> {
    if container == Container::Flac {
        vec![TagEdit {
            label: "BPM",
            value: "128.00".into(),
        }]
    } else {
        vec![
            TagEdit {
                label: "TBPM",
                value: "128".into(),
            },
            TagEdit {
                label: "TXXX:BPM",
                value: "128.00".into(),
            },
        ]
    }
}

/// WAV, AIFF and FLAC files directly in `dir`, by name.
fn audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["wav", "wave", "aif", "aiff", "aifc", "flac"]
                    .contains(&e.to_ascii_lowercase().as_str())
            })
        })
        .collect();
    files.sort();
    files
}

/// The source as a matrix fixture: its blocks with the rules' fates and its samples (read
/// straight from the audio chunk, or decoded by symphonia for FLAC).
fn fixture_of(name: &'static str, bytes: Vec<u8>, input: &Parsed) -> Result<Fixture, String> {
    let (sample_rate, channels, source, ext) = if input.container == Container::Flac {
        let info_block = input
            .find(Kind::FlacBlock, "STREAMINFO")
            .ok_or("no STREAMINFO")?;
        let info = streaminfo(&info_block.bytes)?;
        let decoded = decode(&bytes, "flac")?;
        let data = decoded.as_bits(info.bits);
        let source = Samples::Int {
            bits: info.bits,
            data,
        };
        (info.sample_rate, u16::from(info.channels), source, "flac")
    } else {
        let stored = iff_pcm(input)?;
        let ext = if matches!(input.container, Container::Wave | Container::Rf64) {
            "wav"
        } else {
            "aiff"
        };
        (stored.sample_rate, stored.channels, stored.samples, ext)
    };
    let float = matches!(source, Samples::Float(_));
    let expected = input
        .blocks
        .iter()
        .map(|b| BlockExpect {
            listed: b.listed(),
            expect: rule(float, &b.listed()),
        })
        .collect();
    Ok(Fixture {
        name,
        container: input.container,
        ext,
        bytes,
        frames: source.len() / usize::from(channels.max(1)),
        sample_rate,
        channels,
        bits: source.bits(),
        source,
        expected,
        symphonia: Symphonia::Decodes,
    })
}

/// What one file showed.
enum Outcome {
    /// Rendered and checked; the summary.
    Passed(String),
    /// The renderer refused it, for this reason.
    Refused(String),
}

/// Whether `e` is a renderer refusal of the file (rather than a failure of the writer).
fn is_refusal(e: &Error) -> bool {
    matches!(
        e,
        Error::WouldClip { .. }
            | Error::NotDjSafe { .. }
            | Error::UnsupportedFormat { .. }
            | Error::UnsupportedChannels { .. }
            | Error::DrmProtected { .. }
    )
}

/// One line about the source: format, chunk or block order, block fates, what follows the
/// last chunk, the tag.
fn describe(fx: &Fixture, input: &Parsed, edits: &[TagEdit]) -> String {
    let count = |e: &str| fx.expected.iter().filter(|b| b.expect.name() == e).count();
    let flac = fx.container == Container::Flac;
    let tag = match (edited_tag(input, edits), tags_not_added(input)) {
        (Some(_), _) if flac => "Vorbis comment edited".into(),
        (Some(_), _) => format!(
            "ID3v2.{} edited",
            input.tags.first().map_or(0, |t| t.version)
        ),
        (None, Some(why)) => format!("tag not edited ({why})"),
        (None, None) if flac => "no Vorbis comment".into(),
        (None, None) => "no ID3 tag".into(),
    };
    let layout: Vec<&str> = input
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, Kind::Chunk | Kind::FlacBlock))
        .map(|b| b.id.trim_end())
        .collect();
    let mut tail = Vec::new();
    if let Some(t) = input.blocks.iter().find(|b| b.kind == Kind::Trailing) {
        tail.push(format!(
            "{} B after the last chunk ({} inside the container)",
            t.bytes.len(),
            input.stray_in_container
        ));
    }
    let unpadded = input
        .blocks
        .iter()
        .filter(|b| b.kind == Kind::Chunk && b.bytes.len() % 2 == 1 && b.pad.is_none())
        .count();
    if unpadded > 0 {
        tail.push(format!("{unpadded} missing pad byte restored"));
    }
    let tail = if tail.is_empty() {
        String::new()
    } else {
        format!("; {}", tail.join(", "))
    };
    format!(
        "{:?} {}-bit {} Hz {} ch, {} frames, [{}]; blocks {} carried, {} patched, {} replaced, \
         {} dropped{tail}; {tag}",
        fx.container,
        fx.bits,
        fx.sample_rate,
        fx.channels,
        fx.frames,
        layout.join(" "),
        count("carried"),
        count("patched"),
        count("replaced"),
        count("dropped"),
    )
}

/// Renders, checks, then changes a copy in place and undoes it.
fn check_file(path: &Path, scratch: &Path) -> Result<Outcome, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let input = parse(&bytes).map_err(|e| format!("the source does not parse: {e}"))?;
    let name: &'static str = Box::leak(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into_boxed_str(),
    );
    let fx = fixture_of(name, bytes, &input)?;
    let edits = bpm_edits(fx.container);
    let request = render_request(&ARGS, &edits);
    let opts = TxnOptions {
        sidecar: false,
        ..TxnOptions::new(scratch.join("backups"))
    };
    let cancel = AtomicBool::new(false);
    let report = match txn::apply_to_folder(path, &scratch.join("out"), &request, &opts, &cancel) {
        Ok(report) => report,
        Err(e) if is_refusal(&e) => return Ok(Outcome::Refused(format!("{:?}: {e}", e.kind()))),
        Err(e) => return Err(format!("render failed: {e}")),
    };
    let out = std::fs::read(&report.output).map_err(|e| e.to_string())?;
    let applied = Applied {
        tags_added: report.render.tags_added,
    };
    check_output(&fx, &out, &ARGS, &edits, applied)?;
    let summary = describe(&fx, &input, &edits);
    drop(out);

    let copy_dir = scratch.join("in-place");
    std::fs::create_dir_all(&copy_dir).map_err(|e| e.to_string())?;
    let copy = copy_dir.join(fx.name);
    std::fs::write(&copy, &fx.bytes).map_err(|e| e.to_string())?;
    let in_place = txn::apply_in_place(
        &copy,
        &request,
        &TxnOptions::new(scratch.join("backups")),
        &cancel,
    )
    .map_err(|e| format!("in place: {e}"))?;
    if in_place.output_blake3 != report.output_blake3 {
        return Err("the in-place output differs from the copy into a folder".into());
    }
    txn::undo(&copy, &scratch.join("backups")).map_err(|e| format!("undo: {e}"))?;
    let restored = std::fs::read(&copy).map_err(|e| e.to_string())?;
    if restored != fx.bytes {
        return Err("undo did not restore the original bytes".into());
    }
    if txn::sidecar_path(&copy).exists() {
        return Err("undo left the sidecar".into());
    }
    Ok(Outcome::Passed(format!(
        "{summary}; in place + undo byte-identical"
    )))
}

#[test]
fn real_files_render_without_loss() {
    let Some(dir) = std::env::var_os(ENV).filter(|v| !v.is_empty()) else {
        eprintln!("real_files_render_without_loss: skipped; set {ENV}=<folder> to run it");
        return;
    };
    let files = audio_files(Path::new(&dir));
    assert!(
        !files.is_empty(),
        "no WAV, AIFF or FLAC file in {}",
        dir.display()
    );
    let (mut passed, mut refused, mut failed) = (0, 0, Vec::new());
    for (i, path) in files.iter().enumerate() {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let scratch = tempfile::tempdir().expect("temp folder");
        let start = Instant::now();
        let result = check_file(path, scratch.path());
        let secs = start.elapsed().as_secs_f64();
        let line = match result {
            Ok(Outcome::Passed(s)) => {
                passed += 1;
                format!("ok       {s}")
            }
            Ok(Outcome::Refused(why)) => {
                refused += 1;
                format!("refused  {why}")
            }
            Err(e) => {
                failed.push(name.to_string());
                format!("FAILED   {e}")
            }
        };
        eprintln!("{:>2}/{} {name}: {line} ({secs:.1} s)", i + 1, files.len());
    }
    eprintln!(
        "real files: {} checked, {passed} passed, {refused} refused, {} failed",
        files.len(),
        failed.len()
    );
    assert!(failed.is_empty(), "failed: {failed:?}");
}
