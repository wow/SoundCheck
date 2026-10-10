//! `sc-cli xml`: a rekordbox XML of the files named, exported or not. Every file is analysed
//! (the cache is used) with the grid edits saved in the app applied. A file with a saved edit
//! or confirmation takes its grid from that, the user's latest word; otherwise a file
//! SoundCheck exported takes the grid it exported, read back from its sidecar (schema 2) as long
//! as the file is still the bytes that export wrote; any other file takes its analysed grid,
//! withheld when it needs review and was not confirmed, as an export would withhold it. The
//! files named need no Library opt-in (naming them is the choice), but only files with a grid
//! the XML may carry are listed, each named by its own title and artist tags.
//!
//! The XML goes to `--out` or to stdout. `--out` must end in `.xml` and never replaces a file
//! SoundCheck did not write (an audio file matched by a glob, another app's XML): such a run is
//! refused before anything is analysed. One line per file says what the XML carries for it, or
//! why it is not listed (on stderr when the XML goes to stdout).

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use sc_core::plan::{Codec, DecideSettings};
use sc_core::{Bpm, Error};
use sc_engine::export::{report_rows, xml_tracks};
use sc_engine::{AnalyzeReport, BatchRow, XmlSelect, apply_saved, decide};
use sc_io::edits::EditStore;
use sc_io::rekordbox::is_soundcheck_xml;
use sc_io::txn::sidecar::{self, SidecarDoc};
use sc_io::txn::{hash_file, hex};

use crate::AnalysisArgs;
use crate::process::UNVERIFIED;

/// The flags of `xml`.
#[derive(clap::Args)]
pub struct XmlArgs {
    /// Audio files, exported or only analysed.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Write the XML to this file, which must end in `.xml` (an XML SoundCheck wrote there
    /// before is replaced; any other file is left alone and the run refused), instead of
    /// printing it.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Files analysed at once (default: a quarter of the logical cores, at most 4).
    #[arg(long)]
    jobs: Option<usize>,
    #[command(flatten)]
    analysis: AnalysisArgs,
}

/// `sc-cli xml`: returns how many files failed (1 for a refused `--out`).
///
/// # Errors
/// An unreachable cache or edits folder, or an XML that could not be written.
pub fn run_xml(args: &XmlArgs) -> anyhow::Result<usize> {
    if let Some(out) = &args.out
        && let Some((why, what_to_do)) = out_refusal(out)
    {
        eprintln!(
            "{}: not written\n  why: {why}\n  what to do: {what_to_do}",
            out.display()
        );
        return Ok(1);
    }
    let edits = EditStore::open(EditStore::default_dir()?);
    let settings = crate::cached_batch(&args.analysis, args.jobs)?;
    let mut decide_settings = DecideSettings::dj();
    decide_settings.bpm_range = settings.analysis.bpm_range;
    let mut rows = Vec::with_capacity(args.files.len());
    let mut failed = 0;
    crate::run_in_order(&settings, &args.files, &mut |file, outcome| {
        rows.push(match outcome {
            Ok(report) => row_of(file, report, &edits, &decide_settings),
            Err(err) => {
                failed += 1;
                failed_row(file, &err)
            }
        });
    });
    let tracks = xml_tracks(&rows, XmlSelect::All);
    let now = std::time::SystemTime::now();
    let playlist = sc_io::rekordbox::playlist_name(now);
    let bytes = sc_io::rekordbox::xml_bytes(&tracks, sc_core::VERSION, &playlist);
    if let Some(out) = &args.out {
        sc_io::artefacts::write_artefact(out, &bytes, is_soundcheck_xml)?;
    } else {
        std::io::stdout().lock().write_all(&bytes)?;
    }
    let mut lines: Vec<String> = Vec::new();
    for (row, report) in rows.iter().zip(report_rows(&rows, XmlSelect::All)) {
        let mut line = format!("{}: {}; {}", row.file.display(), report.action, report.grid);
        if let Some(bpm) = report.bpm {
            let _ = write!(line, "; {:.2} BPM", Bpm::written(bpm).0);
        }
        for note in &row.notes {
            line.push_str("; ");
            line.push_str(note);
        }
        lines.push(line);
    }
    let left_out = rows.len() - tracks.len() - failed;
    let counts = format!(
        "{} tracks, {left_out} not listed (no grid the XML may carry), {failed} failed; playlist \"{playlist}\"",
        tracks.len()
    );
    lines.push(match &args.out {
        Some(out) => format!("rekordbox XML: {} ({counts})", out.display()),
        None => counts,
    });
    for line in lines {
        if args.out.is_none() {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    Ok(failed)
}

/// Why `out` may not be written, and what to do: it does not end in `.xml`, or something that
/// is not an XML SoundCheck wrote is there.
fn out_refusal(out: &Path) -> Option<(String, String)> {
    let is_xml = out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
    if !is_xml {
        return Some((
            "the rekordbox XML is written only to a file ending in .xml".to_owned(),
            "name a file ending in .xml, for example --out soundcheck-rekordbox.xml".to_owned(),
        ));
    }
    let meta = std::fs::symlink_metadata(out).ok()?;
    let ours = meta.is_file()
        && std::fs::File::open(out).is_ok_and(|f| {
            use std::io::Read;
            let mut head = Vec::new();
            f.take(512).read_to_end(&mut head).is_ok() && is_soundcheck_xml(&head)
        });
    (!ours).then(|| {
        (
            "a file is already there that SoundCheck did not write; it is never replaced"
                .to_owned(),
            "name a new file, or move that one away first".to_owned(),
        )
    })
}

/// The row of an analysed file: its saved edit when there is one, else its export's grid when
/// the file is still what that export wrote, else its analysis.
fn row_of(
    file: &Path,
    mut report: AnalyzeReport,
    edits: &EditStore,
    decide_settings: &DecideSettings,
) -> BatchRow {
    let edit = apply_saved(&mut report.record, edits);
    let record = &report.record;
    let codec = Codec::from_path(file);
    let exported = sidecar_of(file);
    let mut row = match &exported {
        Some(doc) if !(edit.edited || edit.confirmed) => BatchRow::from_sidecar(file, doc, record),
        _ => None,
    }
    .unwrap_or_else(|| {
        let plan = decide(record, codec, decide_settings, edit.confirmed);
        BatchRow::analysed(file, record, &plan, edit)
    });
    if edit.edited || edit.confirmed {
        row.notes
            .push("grid from the edit saved in the app".to_owned());
    } else if exported.is_none() && sidecar::sidecar_path(file).exists() {
        row.notes
            .push("changed since its export; analysed again".to_owned());
    }
    if matches!(codec, Codec::Mp3 | Codec::Aac) {
        row.notes.push(UNVERIFIED.to_owned());
    }
    row
}

/// The sidecar of `file` when SoundCheck exported it and the file is still the bytes that
/// export wrote.
fn sidecar_of(file: &Path) -> Option<SidecarDoc> {
    let doc: SidecarDoc = sidecar::read(&sidecar::sidecar_path(file)).ok()?;
    doc.export.as_ref()?;
    let output = doc.output.as_ref()?;
    let (bytes, hash) = hash_file(file).ok()?;
    (bytes == output.bytes && hex(&hash) == output.blake3).then_some(doc)
}

/// A failed row, its error printed.
fn failed_row(file: &Path, err: &Error) -> BatchRow {
    eprintln!("sc-cli: {}: {err}", file.display());
    let mut row = BatchRow::failed(file, None);
    row.notes.push(err.to_string());
    row
}
