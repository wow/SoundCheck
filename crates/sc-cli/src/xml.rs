//! `sc-cli xml`: a rekordbox XML of the files named, exported or not. A file SoundCheck exported
//! is read back from its sidecar (schema 2) as long as the file is still the bytes that export
//! wrote: the grid as exported, in the file's samples. Any other file is analysed (the cache is
//! used) with the grid edits saved in the app applied, and its grid is withheld when it needs
//! review and was not confirmed, as an export would withhold it. Every file named is listed.
//! The XML goes to `--out` (replaced atomically) or to stdout; one line per file says what it
//! carries (on stderr when the XML goes to stdout).

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use sc_core::plan::{Codec, DecideSettings};
use sc_core::{Bpm, Error};
use sc_engine::export::{report_rows, xml_tracks};
use sc_engine::{BatchRow, XmlSelect, apply_saved, decide};
use sc_io::edits::EditStore;
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
    /// Write the XML to this file (replacing it) instead of printing it.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Files analysed at once (default: a quarter of the logical cores, at most 4).
    #[arg(long)]
    jobs: Option<usize>,
    #[command(flatten)]
    analysis: AnalysisArgs,
}

/// `sc-cli xml`: returns how many files failed.
///
/// # Errors
/// An unreachable cache or edits folder, or an XML that could not be written.
pub fn run_xml(args: &XmlArgs) -> anyhow::Result<usize> {
    let mut rows: Vec<Option<BatchRow>> = args.files.iter().map(|f| exported_row(f)).collect();
    let todo: Vec<PathBuf> = args
        .files
        .iter()
        .zip(&rows)
        .filter(|(_, row)| row.is_none())
        .map(|(f, _)| f.clone())
        .collect();
    let mut failed = 0;
    if !todo.is_empty() {
        let mut analysed = analyse(args, &todo)?.into_iter();
        for slot in rows.iter_mut().filter(|r| r.is_none()) {
            *slot = analysed.next().map(|outcome| {
                outcome.unwrap_or_else(|row| {
                    failed += 1;
                    row
                })
            });
        }
    }
    let rows: Vec<BatchRow> = rows.into_iter().flatten().collect();
    let tracks = xml_tracks(&rows, XmlSelect::All);
    let bytes = sc_io::rekordbox::xml_bytes(&tracks, sc_core::VERSION);
    let to_stdout = args.out.is_none();
    if let Some(out) = &args.out {
        sc_io::artefacts::write_file(out, &bytes)?;
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
    let with_tempo = tracks.iter().filter(|t| t.tempo.is_some()).count();
    let summary = match &args.out {
        Some(out) => format!(
            "rekordbox XML: {} ({} tracks, {with_tempo} with a grid; {failed} failed)",
            out.display(),
            tracks.len()
        ),
        None => format!(
            "{} tracks, {with_tempo} with a grid; {failed} failed",
            tracks.len()
        ),
    };
    lines.push(summary);
    for line in lines {
        if to_stdout {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    Ok(failed)
}

/// The row of `file` from its sidecar, when SoundCheck exported it and the file is still the
/// bytes that export wrote.
fn exported_row(file: &Path) -> Option<BatchRow> {
    let doc: SidecarDoc = sidecar::read(&sidecar::sidecar_path(file)).ok()?;
    let output = doc.output.as_ref()?;
    let (bytes, hash) = hash_file(file).ok()?;
    if bytes != output.bytes || hex(&hash) != output.blake3 {
        return None;
    }
    let mut row = BatchRow::from_sidecar(file, &doc)?;
    if matches!(Codec::from_path(file), Codec::Mp3 | Codec::Aac) {
        row.notes.push(UNVERIFIED.to_owned());
    }
    Some(row)
}

/// The rows of `files` (none of them exported as they are now), analysed in order.
fn analyse(args: &XmlArgs, files: &[PathBuf]) -> anyhow::Result<Vec<Result<BatchRow, BatchRow>>> {
    let settings = crate::cached_batch(&args.analysis, args.jobs)?;
    let edits = EditStore::open(EditStore::default_dir()?);
    let mut decide_settings = DecideSettings::dj();
    decide_settings.bpm_range = settings.analysis.bpm_range;
    let mut rows = Vec::with_capacity(files.len());
    crate::run_in_order(&settings, files, &mut |file, outcome| {
        rows.push(match outcome {
            Ok(mut report) => {
                let edit = apply_saved(&mut report.record, &edits);
                let codec = Codec::from_path(file);
                let plan = decide(&report.record, codec, &decide_settings, edit.confirmed);
                let mut row = BatchRow::analysed(file, &report.record, &plan, edit);
                if sidecar::sidecar_path(file).exists() {
                    row.notes
                        .push("changed since its export; analysed again".to_owned());
                }
                if matches!(codec, Codec::Mp3 | Codec::Aac) {
                    row.notes.push(UNVERIFIED.to_owned());
                }
                Ok(row)
            }
            Err(err) => Err(failed_row(file, &err)),
        });
    });
    Ok(rows)
}

/// A failed row, its error printed.
fn failed_row(file: &Path, err: &Error) -> BatchRow {
    eprintln!("sc-cli: {}: {err}", file.display());
    let mut row = BatchRow::failed(file, None);
    row.notes.push(err.to_string());
    row
}
