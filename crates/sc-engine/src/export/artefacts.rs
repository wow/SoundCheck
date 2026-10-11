//! A batch's artefacts: the rekordbox XML that carries the grids (`soundcheck-rekordbox.xml`,
//! [`sc_io::rekordbox`]) and the grid report (`grid-report.csv`, [`sc_io::report`]), built from
//! one [`BatchRow`] per file in the order the files were given.
//!
//! **Which files the XML lists.** Only files with a grid it may write: a grid that needs review
//! and was not confirmed is withheld, as it is from the tags, and only 4/4 grids get a `TEMPO`.
//! Importing a track without one could only change its information in rekordbox, or clear the
//! grid rekordbox has. A file written or left to the XML is listed at the path it has now (the
//! copy, for an export to a folder), spelled as its folders list its names
//! ([`sc_io::rekordbox::Speller`]), since rekordbox matches a collection entry by that location,
//! and named by its own title and artist tags. Library tracks are already in a DJ app, whose
//! grid importing the XML would replace, so in a batch ([`XmlSelect::Batch`]) a Library row is
//! listed only when the user opted it in by confirming its grid in SoundCheck. Files named one
//! by one (`sc-cli xml`, [`XmlSelect::All`]) need no opt-in: naming them is the choice. The
//! report keeps every row, with why it is not in the XML.
//!
//! MP3 and AAC rows are marked unverified in the report: the position of bar 1 relative to the
//! encoder delay rekordbox applies has not been checked yet.

use std::path::{Path, PathBuf};

use sc_core::analysis::AnalysisRecord;
use sc_core::export::{
    BatchMode, Cut, ExportOutcome, ExportPlan, GridCheck, GridCheckSkip, XmlGrid, XmlTrackInfo,
};
use sc_core::plan::{Codec, Plan};
use sc_core::{Result, SampleIndex, Seconds};
use sc_io::rekordbox::{Speller, Tempo, TempoWithheld, XmlTrack};
use sc_io::report::ReportRow;
use sc_io::txn::sidecar::SidecarDoc;

use super::plan::grid_trusted;
use super::process::{xml_grid, xml_info};
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
    /// Its title tag.
    pub title: Option<String>,
    /// Its artist tag.
    pub artist: Option<String>,
}

impl RowTrack {
    fn new(path: &Path, duration: Seconds, codec: Codec, xml: XmlTrackInfo) -> Self {
        Self {
            path: path.to_path_buf(),
            duration,
            codec,
            grid: xml.grid,
            title: xml.title,
            artist: xml.artist,
        }
    }
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
    /// The check of the exported grid against the written file's analysis: set by the caller
    /// for a file written ([`crate::ProcessDone::grid_check`]), read from the sidecar for one
    /// exported earlier, "not checked" for one left to the XML; `None` otherwise.
    pub grid_check: Option<GridCheck>,
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
        xml: XmlTrackInfo,
    ) -> Self {
        Self {
            file: file.to_path_buf(),
            mode: Some(mode),
            action: RowAction::Written,
            gain_db: Some(plan.gain_db),
            cut: cut_of(plan),
            track: Some(RowTrack::new(
                output,
                SampleIndex(frames_out).to_seconds(sample_rate.max(1)),
                Codec::from_path(output),
                xml,
            )),
            grid_check: None,
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
        xml: XmlTrackInfo,
    ) -> Self {
        let (action, track, grid_check) = match outcome {
            ExportOutcome::XmlOnly { .. } => (
                RowAction::XmlOnly,
                Some(RowTrack::new(file, duration, codec, xml)),
                Some(GridCheck::NotChecked {
                    reason: GridCheckSkip::XmlOnly,
                }),
            ),
            ExportOutcome::Skip { .. } | ExportOutcome::Write { .. } => {
                (RowAction::Skipped, None, None)
            }
        };
        Self {
            file: file.to_path_buf(),
            mode: Some(mode),
            action,
            gain_db: None,
            cut: None,
            track,
            grid_check,
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
            grid_check: None,
            notes: Vec::new(),
        }
    }

    /// A file an earlier export wrote, from its sidecar (`doc`, whose `export` must be set and
    /// whose output must still be the file's bytes: the caller checks), named by `record`'s
    /// tags (the file's own analysis); `None` for a sidecar without an export.
    #[must_use]
    pub fn from_sidecar(file: &Path, doc: &SidecarDoc, record: &AnalysisRecord) -> Option<Self> {
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
            track: Some(RowTrack::new(
                file,
                SampleIndex(doc.render.frames_out).to_seconds(rate.max(1)),
                Codec::from_path(file),
                xml_info(record, grid),
            )),
            grid_check: export.grid_check.clone(),
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
            track: Some(RowTrack::new(
                file,
                record.duration,
                Codec::from_path(file),
                xml_info(record, xml_grid(record, 0, grid_trusted(plan), edit)),
            )),
            grid_check: None,
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
    /// Every row with a file and a grid to list, Library rows included: the user named the
    /// files (`sc-cli xml`), which is the opt-in.
    All,
    /// A batch's rows: Prepare rows, and Library rows whose grid the user confirmed.
    Batch,
}

/// Whether a row's file is in the XML, with its tempo, or why not.
enum Listing {
    Listed(Tempo),
    NotListed(String),
}

fn listing(row: &BatchRow, select: XmlSelect) -> Listing {
    let not = |why: &str| Listing::NotListed(format!("not in the XML: {why}"));
    let Some(track) = &row.track else {
        return Listing::NotListed("not in the XML".to_owned());
    };
    if select == XmlSelect::Off {
        return not("the XML is off");
    }
    let tempo = match Tempo::of(&track.grid) {
        Ok(tempo) => tempo,
        Err(TempoWithheld::NeedsReview) => return not("grid needs review"),
        Err(TempoWithheld::Absent) => return not("no grid"),
        Err(TempoWithheld::Meter(meter)) => {
            return not(&format!("meter {meter} (the XML carries 4/4 grids only)"));
        }
        Err(TempoWithheld::InvalidTempo) => return not("no valid tempo"),
    };
    let confirmed = track.grid.grid().is_some_and(|(g, _)| g.confirmed);
    if select == XmlSelect::Batch && row.mode == Some(BatchMode::Library) && !confirmed {
        return not("Library rows need a confirmed grid");
    }
    Listing::Listed(tempo)
}

/// The XML's tracks, in the order of `rows`: every row `select` lists, at its path as its
/// folders spell it (its absolute path when they cannot be read).
#[must_use]
pub fn xml_tracks(rows: &[BatchRow], select: XmlSelect) -> Vec<XmlTrack> {
    let mut speller = Speller::new();
    rows.iter()
        .filter_map(|row| match (listing(row, select), &row.track) {
            (Listing::Listed(tempo), Some(track)) => Some(XmlTrack {
                path: speller.spell(&track.path),
                title: track.title.clone(),
                artist: track.artist.clone(),
                duration: track.duration,
                tempo,
            }),
            _ => None,
        })
        .collect()
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
                grid_check: row.grid_check.as_ref().map(ToString::to_string),
                notes: row.notes.clone(),
            }
        })
        .collect()
}

/// What the XML carries for `row`, or why it carries nothing.
fn grid_text(row: &BatchRow, select: XmlSelect) -> String {
    match listing(row, select) {
        Listing::NotListed(why) => why,
        Listing::Listed(t) => {
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
    }
}

/// Where a batch's artefacts were written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artefacts {
    /// The rekordbox XML; `None` when it is off.
    pub xml: Option<PathBuf>,
    /// The grid report.
    pub report: PathBuf,
    /// Tracks the XML lists (each with its grid).
    pub listed: usize,
}

/// Writes the grid report of `rows` and, unless `select` is [`XmlSelect::Off`], the rekordbox
/// XML (its playlist named `playlist`, [`sc_io::rekordbox::playlist_name`]) into `dir`, each
/// replacing an older one SoundCheck wrote atomically; anything else of that name is left as
/// it is.
///
/// # Errors
/// [`sc_core::Error::AlreadyExists`] naming a file there that SoundCheck did not write;
/// [`sc_core::Error::Io`] naming the file that could not be written.
pub fn write_artefacts(
    dir: &Path,
    rows: &[BatchRow],
    select: XmlSelect,
    playlist: &str,
) -> Result<Artefacts> {
    use sc_io::artefacts::write_artefact;
    let tracks = xml_tracks(rows, select);
    let xml = if select == XmlSelect::Off {
        None
    } else {
        let path = dir.join(sc_io::rekordbox::XML_FILE_NAME);
        let bytes = sc_io::rekordbox::xml_bytes(&tracks, sc_core::VERSION, playlist);
        write_artefact(&path, &bytes, sc_io::rekordbox::is_soundcheck_xml)?;
        Some(path)
    };
    let report = dir.join(sc_io::report::REPORT_FILE_NAME);
    let csv = sc_io::report::csv_bytes(&report_rows(rows, select));
    write_artefact(&report, &csv, sc_io::report::is_soundcheck_report)?;
    Ok(Artefacts {
        xml,
        report,
        listed: tracks.len(),
    })
}

#[cfg(test)]
mod tests;
