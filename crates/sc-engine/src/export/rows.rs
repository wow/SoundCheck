//! One [`BatchRow`] per file for a batch's rekordbox XML and grid report, with its notes worded,
//! and the checks that keep those artefacts from replacing what SoundCheck did not write: what
//! the command line and the app share, so both write the same report for the same batch.

use std::path::{Path, PathBuf};

use sc_core::Error;
use sc_core::export::{BatchMode, ExportOutcome, ExportPlan, XmlTrackInfo};
use sc_core::plan::{Codec, DecideSettings};
use sc_core::{Result, Seconds};
use sc_io::edits::EditStore;
use sc_io::rekordbox::is_soundcheck_xml;
use sc_io::txn::sidecar::{self, SidecarDoc};
use sc_io::txn::{TxnReport, hash_file, hex};

use super::ProcessDone;
use super::artefacts::merge::{listed_in, reported_in};
use super::artefacts::{BatchRow, XmlSelect, is_listed, report_rows};
use super::wording::{GRID_WITHHELD, UNVERIFIED, notice_text, skip_why, xml_only_text};
use crate::analyze::AnalyzeReport;
use crate::{apply_saved, decide};

/// The row of a file this batch wrote.
#[must_use]
pub fn written_row(
    file: &Path,
    mode: BatchMode,
    plan: &ExportPlan,
    report: &TxnReport,
    done: &ProcessDone,
) -> BatchRow {
    let mut row = BatchRow::written(
        file,
        mode,
        plan,
        &done.output,
        report.render.frames_out,
        report.render.sample_rate_hz,
        done.xml.clone(),
    );
    row.grid_check = Some(done.grid_check.clone());
    row.notes
        .extend(plan.notices.iter().map(|n| notice_text(*n)));
    if plan.grid_withheld {
        row.notes.push(GRID_WITHHELD.to_owned());
    }
    if let Some(why) = &report.render.tags_not_added {
        row.notes.push(format!("tags not added: {why}"));
    }
    row.notes
        .extend(report.notes.iter().chain(&done.notes).cloned());
    row
}

/// The row of a file this batch did not write.
#[must_use]
pub fn not_written_row(
    file: &Path,
    mode: BatchMode,
    outcome: &ExportOutcome,
    duration: Seconds,
    codec: Codec,
    xml: &XmlTrackInfo,
) -> BatchRow {
    let mut row = BatchRow::not_written(file, mode, outcome, duration, codec, xml.clone());
    match *outcome {
        ExportOutcome::XmlOnly { reason } => row.notes.push(xml_only_text(reason)),
        ExportOutcome::Skip { reason } => row.notes.push(skip_why(reason)),
        ExportOutcome::Write { .. } => {}
    }
    row
}

/// The row of a file refused, failed or cancelled.
#[must_use]
pub fn failed_row(file: &Path, mode: Option<BatchMode>, error: &Error) -> BatchRow {
    let mut row = BatchRow::failed(file, mode);
    row.notes.push(error.to_string());
    row
}

/// The row of an analysed file named for the rekordbox XML: its saved edit when there is one,
/// else its export's grid when the file is still what that export wrote, else its analysis. The
/// analysis comes with the user's edit not yet applied; `decide_settings` decide whether the
/// grid needs review.
#[must_use]
pub fn analysed_row(
    file: &Path,
    mut report: AnalyzeReport,
    edits: Option<&EditStore>,
    decide_settings: &DecideSettings,
) -> BatchRow {
    let edit = edits
        .map(|store| apply_saved(&mut report.record, store))
        .unwrap_or_default();
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

/// For each of `rows` the XML selected by `select` does not list, its index and why.
#[must_use]
pub fn not_listed(rows: &[BatchRow], select: XmlSelect) -> Vec<(usize, String)> {
    rows.iter()
        .zip(report_rows(rows, select))
        .enumerate()
        .filter(|(_, (row, _))| !is_listed(row, select))
        .map(|(i, (row, report))| {
            let mut why = report.grid;
            for note in &row.notes {
                why.push_str("; ");
                why.push_str(note);
            }
            (i, why)
        })
        .collect()
}

/// An artefact name in a folder taken by a file SoundCheck did not write.
#[derive(Debug)]
pub struct ArtefactConflict {
    /// The file there.
    pub path: PathBuf,
    /// The artefact's name.
    pub name: &'static str,
    /// [`Error::AlreadyExists`], or why it could not be read to check it.
    pub error: Error,
}

/// The first artefact name in the folder `dir` that a batch could not write, because a file
/// SoundCheck did not write is there, or one of its own that cannot be read whole, whose entries a
/// new one could not keep. Asked before anything is exported.
#[must_use]
pub fn artefact_conflict(dir: &Path) -> Option<ArtefactConflict> {
    use sc_io::artefacts::check_replaceable;
    type IsOurs = fn(&[u8]) -> bool;
    let checks: [(&'static str, IsOurs); 2] = [
        (
            sc_io::rekordbox::XML_FILE_NAME,
            sc_io::rekordbox::is_soundcheck_xml,
        ),
        (
            sc_io::report::REPORT_FILE_NAME,
            sc_io::report::is_soundcheck_report,
        ),
    ];
    checks.into_iter().find_map(|(name, is_ours)| {
        let path = dir.join(name);
        let checked: Result<()> = check_replaceable(&path, is_ours).and_then(|()| {
            if name == sc_io::rekordbox::XML_FILE_NAME {
                listed_in(dir).map(|_| ())
            } else {
                reported_in(dir).map(|_| ())
            }
        });
        checked
            .err()
            .map(|error| ArtefactConflict { path, name, error })
    })
}

/// Why a rekordbox XML may not be written to a file the user named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlOutRefusal {
    /// It does not end in `.xml`: a glob could otherwise name an audio file.
    NotXml,
    /// Something that is not an XML SoundCheck wrote is there.
    Taken,
}

/// Why the rekordbox XML may not be written to `out`, if so.
#[must_use]
pub fn xml_out_refusal(out: &Path) -> Option<XmlOutRefusal> {
    let is_xml = out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
    if !is_xml {
        return Some(XmlOutRefusal::NotXml);
    }
    let meta = std::fs::symlink_metadata(out).ok()?;
    let ours = meta.is_file()
        && std::fs::File::open(out).is_ok_and(|f| {
            use std::io::Read;
            let mut head = Vec::new();
            f.take(512).read_to_end(&mut head).is_ok() && is_soundcheck_xml(&head)
        });
    (!ours).then_some(XmlOutRefusal::Taken)
}
