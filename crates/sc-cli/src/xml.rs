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

use sc_core::plan::DecideSettings;
use sc_core::{Bpm, Error};
use sc_engine::export::{XmlOutRefusal, analysed_row, report_rows, xml_tracks};
use sc_engine::{BatchRow, XmlSelect};
use sc_io::edits::EditStore;
use sc_io::rekordbox::is_soundcheck_xml;

use crate::AnalysisArgs;

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
            Ok(report) => analysed_row(file, report, Some(&edits), &decide_settings),
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
    sc_engine::export::xml_out_refusal(out).map(|refusal| match refusal {
        XmlOutRefusal::NotXml => (
            "the rekordbox XML is written only to a file ending in .xml".to_owned(),
            "name a file ending in .xml, for example --out soundcheck-rekordbox.xml".to_owned(),
        ),
        XmlOutRefusal::Taken => (
            "a file is already there that SoundCheck did not write; it is never replaced"
                .to_owned(),
            "name a new file, or move that one away first".to_owned(),
        ),
    })
}

/// A failed row, its error printed.
fn failed_row(file: &Path, err: &Error) -> BatchRow {
    eprintln!("sc-cli: {}: {err}", file.display());
    sc_engine::export::failed_row(file, None, err)
}
