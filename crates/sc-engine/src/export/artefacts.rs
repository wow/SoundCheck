//! A batch's artefacts: the rekordbox XML that carries the grids (`soundcheck-rekordbox.xml`,
//! [`sc_io::rekordbox`]) and the grid report (`grid-report.csv`, [`sc_io::report`]), built from
//! one [`BatchRow`] per file in the order the files were given.
//!
//! **Which files the XML lists.** A file written or left to the XML is listed at the path it
//! has now (the copy, for an export to a folder), spelled as the file system stores its names,
//! since rekordbox matches a collection entry by that location. In Prepare mode every such file
//! is listed. Library tracks are already in a DJ app, whose grid importing the XML would
//! replace, so a Library row is listed only when the user opted it in by confirming its grid in
//! SoundCheck ([`XmlSelect::Batch`]). Files named one by one (`sc-cli xml`) are all listed
//! ([`XmlSelect::All`]).
//!
//! **Which tracks get a grid.** A grid that needs review and was not confirmed is withheld, as
//! it is from the tags; only 4/4 grids get a `TEMPO`. MP3 and AAC rows are marked unverified in
//! the report: the position of bar 1 relative to the encoder delay rekordbox applies has not
//! been checked yet.

use std::path::{Path, PathBuf};

use sc_core::analysis::AnalysisRecord;
use sc_core::export::{BatchMode, Cut, ExportOutcome, ExportPlan, XmlGrid};
use sc_core::plan::{Codec, Plan};
use sc_core::{Result, SampleIndex, Seconds};
use sc_io::rekordbox::{Tempo, TempoWithheld, XmlTrack};
use sc_io::report::ReportRow;
use sc_io::txn::sidecar::SidecarDoc;

use super::plan::grid_trusted;
use super::process::xml_grid;
use crate::edits::EditState;

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    /// Written by this batch.
    Written,
    /// Left as it is; only the rekordbox XML carries its grid.
    XmlOnly,
    /// Left out.
    Skipped,
    /// Refused or failed.
    Failed,
    /// Written by an earlier export (read back from its sidecar).
    Exported,
    /// Analysed only (never exported, or changed since).
    Analysed,
}

impl RowAction {
    /// The report's word for it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::XmlOnly => "xml only",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Exported => "exported earlier",
            Self::Analysed => "analysed",
        }
    }
}

/// The file a row's XML entry points at, and the grid it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct RowTrack {
    /// The file as it is now: the output of a write, else the file itself.
    pub path: PathBuf,
    /// Its playing time.
    pub duration: Seconds,
    /// Its codec.
    pub codec: Codec,
    /// Its grid, in its samples, or why there is none.
    pub grid: XmlGrid,
}

/// What a batch did with one file, for its rekordbox XML and grid report.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchRow {
    /// The file as given.
    pub file: PathBuf,
    /// The batch mode; `None` for a file only analysed.
    pub mode: Option<BatchMode>,
    /// What happened to it.
    pub action: RowAction,
    /// The gain written, dB.
    pub gain_db: Option<f64>,
    /// Audio cut from its start.
    pub cut: Option<Seconds>,
    /// What the XML may list; `None` for a file skipped or failed.
    pub track: Option<RowTrack>,
    /// The caller's notes (reasons, notices), in order.
    pub notes: Vec<String>,
}

impl BatchRow {
    /// A file this batch wrote: `plan` and the output's frame count and rate.
    #[must_use]
    pub fn written(
        file: &Path,
        mode: BatchMode,
        plan: &ExportPlan,
        output: &Path,
        frames_out: u64,
        sample_rate: u32,
        grid: XmlGrid,
    ) -> Self {
        Self {
            file: file.to_path_buf(),
            mode: Some(mode),
            action: RowAction::Written,
            gain_db: Some(plan.gain_db),
            cut: cut_of(plan),
            track: Some(RowTrack {
                path: output.to_path_buf(),
                duration: SampleIndex(frames_out).to_seconds(sample_rate.max(1)),
                codec: Codec::from_path(output),
                grid,
            }),
            notes: Vec::new(),
        }
    }

    /// A file this batch did not write: left to the XML (listed with `grid`) or skipped.
    #[must_use]
    pub fn not_written(
        file: &Path,
        mode: BatchMode,
        outcome: &ExportOutcome,
        duration: Seconds,
        codec: Codec,
        grid: XmlGrid,
    ) -> Self {
        let (action, track) = match outcome {
            ExportOutcome::XmlOnly { .. } => (
                RowAction::XmlOnly,
                Some(RowTrack {
                    path: file.to_path_buf(),
                    duration,
                    codec,
                    grid,
                }),
            ),
            ExportOutcome::Skip { .. } | ExportOutcome::Write { .. } => (RowAction::Skipped, None),
        };
        Self {
            file: file.to_path_buf(),
            mode: Some(mode),
            action,
            gain_db: None,
            cut: None,
            track,
            notes: Vec::new(),
        }
    }

    /// A file refused or failed.
    #[must_use]
    pub fn failed(file: &Path, mode: Option<BatchMode>) -> Self {
        Self {
            file: file.to_path_buf(),
            mode,
            action: RowAction::Failed,
            gain_db: None,
            cut: None,
            track: None,
            notes: Vec::new(),
        }
    }

    /// A file an earlier export wrote, from its sidecar (`doc`, whose `export` must be set and
    /// whose output must still be the file's bytes: the caller checks); `None` for a sidecar
    /// without an export.
    #[must_use]
    pub fn from_sidecar(file: &Path, doc: &SidecarDoc) -> Option<Self> {
        let export = doc.export.as_ref()?;
        let rate = doc.render.sample_rate_hz;
        let grid = match &export.grid {
            _ if export.plan.grid_withheld => XmlGrid::NeedsReview,
            Some(grid) => XmlGrid::Grid {
                grid: grid.clone(),
                sample_rate: rate,
            },
            None => XmlGrid::Absent,
        };
        Some(Self {
            file: file.to_path_buf(),
            mode: Some(export.settings.batch_mode),
            action: RowAction::Exported,
            gain_db: Some(export.plan.gain_db),
            cut: cut_of(&export.plan),
            track: Some(RowTrack {
                path: file.to_path_buf(),
                duration: SampleIndex(doc.render.frames_out).to_seconds(rate.max(1)),
                codec: Codec::from_path(file),
                grid,
            }),
            notes: Vec::new(),
        })
    }

    /// A file only analysed: `record` with the user's edit applied (`edit`), `plan` decided
    /// with it.
    #[must_use]
    pub fn analysed(file: &Path, record: &AnalysisRecord, plan: &Plan, edit: EditState) -> Self {
        Self {
            file: file.to_path_buf(),
            mode: None,
            action: RowAction::Analysed,
            gain_db: None,
            cut: None,
            track: Some(RowTrack {
                path: file.to_path_buf(),
                duration: record.duration,
                codec: Codec::from_path(file),
                grid: xml_grid(record, 0, grid_trusted(plan), edit),
            }),
            notes: Vec::new(),
        }
    }
}

/// The cut a plan made, when it made one.
fn cut_of(plan: &ExportPlan) -> Option<Seconds> {
    match plan.cut {
        Cut::Cut { seconds, .. } => Some(seconds),
        _ => None,
    }
}

/// Which rows the XML lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlSelect {
    /// No XML.
    Off,
    /// Every row with a file to list.
    All,
    /// A batch's rows: Prepare rows, and Library rows whose grid the user confirmed.
    Batch,
}

/// Whether a row's file is in the XML, with its tempo, or why not.
enum Listing {
    Listed(std::result::Result<Tempo, TempoWithheld>),
    NotListed(&'static str),
}

fn listing(row: &BatchRow, select: XmlSelect) -> Listing {
    let Some(track) = &row.track else {
        return Listing::NotListed("not in the XML");
    };
    let confirmed = track.grid.grid().is_some_and(|(g, _)| g.confirmed);
    match select {
        XmlSelect::Off => Listing::NotListed("not in the XML: the XML is off"),
        XmlSelect::Batch if row.mode == Some(BatchMode::Library) && !confirmed => {
            Listing::NotListed("not in the XML: Library rows need a confirmed grid")
        }
        XmlSelect::All | XmlSelect::Batch => Listing::Listed(Tempo::of(&track.grid)),
    }
}

/// The XML's tracks, in the order of `rows`: every row `select` lists, at its path as the
/// file system spells it (its absolute path when that cannot be read).
#[must_use]
pub fn xml_tracks(rows: &[BatchRow], select: XmlSelect) -> Vec<XmlTrack> {
    rows.iter()
        .filter_map(|row| match (listing(row, select), &row.track) {
            (Listing::Listed(tempo), Some(track)) => Some(XmlTrack {
                path: on_disk(&track.path),
                duration: track.duration,
                tempo: tempo.ok(),
            }),
            _ => None,
        })
        .collect()
}

/// `path` as the file system stores its names, else absolute as given.
fn on_disk(path: &Path) -> PathBuf {
    sc_io::txn::resolve_file(path)
        .ok()
        .or_else(|| std::path::absolute(path).ok())
        .unwrap_or_else(|| path.to_path_buf())
}

/// The report's rows, one per row of `rows`, in order.
#[must_use]
pub fn report_rows(rows: &[BatchRow], select: XmlSelect) -> Vec<ReportRow> {
    rows.iter()
        .map(|row| {
            let grid = row.track.as_ref().and_then(|t| t.grid.grid());
            ReportRow {
                file: row.file.display().to_string(),
                mode: row.mode.map_or("", BatchMode::as_str).to_owned(),
                action: row.action.as_str().to_owned(),
                gain_db: row.gain_db,
                cut: row.cut,
                bpm: grid.map(|(g, _)| g.bpm),
                bar1: grid.map(|(g, rate)| g.bar1.to_seconds(rate.max(1))),
                grid: grid_text(row, select),
                grid_check: None,
                notes: row.notes.clone(),
            }
        })
        .collect()
}

/// What the XML carries for `row`, or why it carries nothing.
fn grid_text(row: &BatchRow, select: XmlSelect) -> String {
    let tempo = match listing(row, select) {
        Listing::NotListed(why) => return why.to_owned(),
        Listing::Listed(tempo) => tempo,
    };
    match tempo {
        Ok(t) => {
            let unverified = row
                .track
                .as_ref()
                .is_some_and(|t| matches!(t.codec, Codec::Mp3 | Codec::Aac));
            let mut text = format!("tempo: beat {} at {:.3} s", t.battito, t.inizio.0);
            if unverified {
                text.push_str(" (unverified: MP3/AAC encoder delay)");
            }
            text
        }
        Err(TempoWithheld::NeedsReview) => "withheld: grid needs review".to_owned(),
        Err(TempoWithheld::Absent) => "no grid".to_owned(),
        Err(TempoWithheld::Meter(meter)) => {
            format!("withheld: meter {meter} (the XML carries 4/4 grids only)")
        }
        Err(TempoWithheld::InvalidTempo) => "withheld: no valid tempo".to_owned(),
    }
}

/// Where a batch's artefacts were written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artefacts {
    /// The rekordbox XML; `None` when it is off.
    pub xml: Option<PathBuf>,
    /// The grid report.
    pub report: PathBuf,
    /// Tracks the XML lists.
    pub listed: usize,
    /// Of those, tracks with a grid (`TEMPO`).
    pub with_tempo: usize,
}

/// Writes the grid report of `rows` and, unless `select` is [`XmlSelect::Off`], the rekordbox
/// XML into `dir`, each replacing an older one atomically.
///
/// # Errors
/// [`sc_core::Error::Io`] naming the file that could not be written.
pub fn write_artefacts(dir: &Path, rows: &[BatchRow], select: XmlSelect) -> Result<Artefacts> {
    let tracks = xml_tracks(rows, select);
    let xml = if select == XmlSelect::Off {
        None
    } else {
        let path = dir.join(sc_io::rekordbox::XML_FILE_NAME);
        let bytes = sc_io::rekordbox::xml_bytes(&tracks, sc_core::VERSION);
        sc_io::artefacts::write_file(&path, &bytes)?;
        Some(path)
    };
    let report = dir.join(sc_io::report::REPORT_FILE_NAME);
    sc_io::artefacts::write_file(
        &report,
        &sc_io::report::csv_bytes(&report_rows(rows, select)),
    )?;
    Ok(Artefacts {
        xml,
        report,
        listed: tracks.len(),
        with_tempo: tracks.iter().filter(|t| t.tempo.is_some()).count(),
    })
}

#[cfg(test)]
mod tests;
