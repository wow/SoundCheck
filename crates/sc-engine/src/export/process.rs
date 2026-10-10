//! PROCESS: exporting one file end to end, as a batch's worker runs it ([`crate::batch::Task`]).
//!
//! 1. The analysis (from the cache when current), with the user's saved grid edit applied, and
//!    the plan under the batch's loudness settings ([`crate::decide()`]).
//! 2. The source read again ([`ExportSource::read`]: headers, chunks, Serato data, hashed),
//!    never taken from an older probe, and the export planned from the cut the renderer makes
//!    ([`super::plan_export_snapped`]). A file that is not written ends here.
//! 3. The write ([`crate::apply_file`]): in place after a backup, or a copy into a folder;
//!    asked to cut exactly the planned (snapped) cut, and refused before anything is replaced
//!    unless the output has the planned frame count (Library: the source's; Prepare: the
//!    source's minus the cut). The sidecar records the plan, the exported grid and the
//!    measurements ([`ExportRecord`]).
//! 4. After the write: the user's grid edit is carried over to the output (bar 1 back by the
//!    cut), and the output is analysed afresh, which replaces its cache entry. A failure here
//!    leaves the file written and becomes a note.

use std::path::{Path, PathBuf};

use sc_core::analysis::{AnalysisRecord, AnalysisSettings, Grid};
use sc_core::export::{
    ExportOutcome, ExportPlan, ExportRecord, ExportSettings, ExportedGrid, Place as ExportPlace,
    SourceMeasurements,
};
use sc_core::plan::{DecideSettings, Plan};
use sc_core::{Error, Result, Seconds};
use sc_io::edits::{EditStore, SavedEdit};

use super::plan::{Bars, ExportInput, ExportSource, bar1_after};
use super::plan_export_snapped;
use crate::analyze::{Analyzer, Progress, Timings};
use crate::batch::EngineEvent;
use crate::edits::{EditState, apply_saved, audio_of, carry_edit};
use crate::txn::{ApplyOptions, ApplyRequest, Place, RecoveryGate, apply_file};
use crate::{CancelToken, decide};

/// How a batch exports its files.
#[derive(Debug, Clone)]
pub struct ProcessSettings {
    /// The loudness settings every file's gain is decided with.
    pub decide: DecideSettings,
    /// The export settings.
    pub export: ExportSettings,
    /// The folder copies go to; required by, and only by, [`ExportPlace::Folder`].
    pub out_dir: Option<PathBuf>,
    /// Where backups and the journal live.
    pub backup_root: PathBuf,
    /// Where the user's grid edits are saved: applied to every analysis and carried over to
    /// every output; `None` applies none.
    pub edits: Option<EditStore>,
    /// The start-up recovery: no file is analysed or written while it runs.
    pub recovery: std::sync::Arc<RecoveryGate>,
}

impl ProcessSettings {
    /// Checks the loudness and export settings and that a folder is given exactly when the
    /// export goes to one.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] naming the problem.
    pub fn validate(&self) -> Result<()> {
        self.decide.validate()?;
        self.export.validate()?;
        match (self.export.place, &self.out_dir) {
            (ExportPlace::InPlace, None) | (ExportPlace::Folder, Some(_)) => Ok(()),
            (ExportPlace::InPlace, Some(dir)) => Err(Error::InvalidArgument(format!(
                "an export in place has no folder, but {} was given",
                dir.display()
            ))),
            (ExportPlace::Folder, None) => Err(Error::InvalidArgument(
                "an export to a folder needs the folder".into(),
            )),
        }
    }

    /// How the transaction writes: Library keeps the modification time (DJ apps then do not
    /// see a changed file), Prepare does not (a new track is analysed again anyway).
    fn apply_options(&self) -> ApplyOptions {
        ApplyOptions {
            place: match &self.out_dir {
                Some(dir) => Place::Folder(dir.clone()),
                None => Place::InPlace,
            },
            backup_root: self.backup_root.clone(),
            keep_mtime: self.export.batch_mode == sc_core::export::BatchMode::Library,
            sidecar: true,
        }
    }
}

/// What became of a written file after the write.
#[derive(Debug, Clone)]
pub struct ProcessDone {
    /// The file written (the source's path in place).
    pub output: PathBuf,
    /// The source's playing time.
    pub duration: Seconds,
    /// The output's own analysis, now in the cache, with the carried grid edit applied; `None`
    /// when it did not finish (see `notes`).
    pub analysis: Option<Box<AnalysisRecord>>,
    /// What the carried grid edit does to the output's grid.
    pub edit: EditState,
    /// The user's grid edit was carried over to the output.
    pub edit_carried: bool,
    /// What did not work after the write; the file stays written.
    pub notes: Vec<String>,
}

/// Exports `path` (the batch's file `file_id`) as `settings` say, sending
/// [`EngineEvent::Processing`] and [`EngineEvent::Written`] through `send` as it goes; returns
/// the file's terminal event: [`EngineEvent::ExportSkipped`] or [`EngineEvent::Done`].
///
/// # Errors
/// The analysis's, the planner's and the transaction's errors ([`Error::Cancelled`] when the
/// batch is cancelled before the output replaces anything); on any error the file is as it was.
pub(crate) fn process_file(
    analyzer: &mut Analyzer,
    file_id: u32,
    path: &Path,
    settings: &ProcessSettings,
    progress: Option<&Progress>,
    send: &dyn Fn(EngineEvent),
) -> Result<EngineEvent> {
    let mut record = analyzer
        .analyze_with(path, &mut Timings::default(), progress)?
        .record;
    let saved = settings
        .edits
        .as_ref()
        .and_then(|store| store.get(&record.path, &audio_of(&record)));
    let edit = settings
        .edits
        .as_ref()
        .map(|store| apply_saved(&mut record, store))
        .unwrap_or_default();
    let mut source = ExportSource::read(path);
    if source.codec.is_writable() {
        source.blake3 = Some(sc_io::txn::hash_file(path)?.1);
    }
    let decided = decide(&record, source.codec, &settings.decide, edit.confirmed);
    let input = ExportInput {
        record: &record,
        plan: &decided,
        decide: &settings.decide,
        source: &source,
    };
    let plan = match plan_export_snapped(path, &input, &settings.export)? {
        ExportOutcome::Write { plan } => plan,
        outcome => {
            return Ok(EngineEvent::ExportSkipped {
                file_id,
                outcome: Box::new(outcome),
                duration: record.duration,
            });
        }
    };
    send(EngineEvent::Processing {
        file_id,
        plan: Box::new(plan.clone()),
    });
    let export = export_record(&record, &decided, &plan, settings, edit);
    let request = ApplyRequest {
        gain_db: plan.gain_db,
        trim_frames: plan.trim_frames,
        trim_snapped_from_frames: plan.trim_snapped_from_frames,
        bits: plan.bits,
        loudness: plan.bext,
        tags: plan.tags.clone(),
        export: Some(export),
        source_blake3: source.blake3,
    };
    let report = apply_file(path, &request, &settings.apply_options(), &analyzer.cancel)?;
    // The output as the caller spells it (the transaction reports the path the file system
    // resolves), so the cache and the grid edits find it under the name the user knows.
    let output = match (&settings.out_dir, report.output.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        (Some(_), None) => report.output.clone(),
        (None, _) => path.to_path_buf(),
    };
    send(EngineEvent::Written {
        file_id,
        report: Box::new(report),
    });
    let carry = Carry {
        saved: saved.as_ref(),
        edit,
        grid: record.grid.as_ref(),
        trim: plan.trim_frames,
    };
    let done = after_write(analyzer, &output, settings, &carry, record.duration);
    Ok(EngineEvent::Done {
        file_id,
        done: Box::new(done),
    })
}

/// What the sidecar records about an export of `record`.
fn export_record(
    record: &AnalysisRecord,
    decided: &Plan,
    plan: &ExportPlan,
    settings: &ProcessSettings,
    edit: EditState,
) -> ExportRecord {
    ExportRecord {
        settings: settings.export,
        decide: settings.decide,
        decided: decided.clone(),
        plan: plan.clone(),
        grid: exported_grid(record, plan, edit),
        source: SourceMeasurements::of(record),
    }
}

/// The grid of `record` as `plan` exports it, in the output's samples; `None` without a grid or
/// when the plan withholds it.
fn exported_grid(
    record: &AnalysisRecord,
    plan: &ExportPlan,
    edit: EditState,
) -> Option<ExportedGrid> {
    if plan.grid_withheld {
        return None;
    }
    let grid = record.grid.as_ref()?;
    let bars = Bars::of(grid, record.spec.sample_rate)?;
    Some(ExportedGrid {
        bar1: sc_core::SampleIndex(grid.anchor.0.saturating_sub(plan.trim_frames)),
        first_bar_line: bar1_after(&bars, plan.trim_frames),
        bpm: grid.bpm.written(),
        bpm_exact: grid.bpm,
        meter: grid.meter.clone(),
        edited: edit.edited,
        confirmed: edit.confirmed,
    })
}

/// The user's grid edit of the source, to carry over to the output.
struct Carry<'a> {
    /// The edit saved for the source, if any.
    saved: Option<&'a SavedEdit>,
    /// What it did to the source's grid.
    edit: EditState,
    /// The source's grid as shown.
    grid: Option<&'a Grid>,
    /// Frames cut from the source's start.
    trim: u64,
}

/// After the write: the grid edit carried over, then the output's own analysis (its cache entry
/// replaced, the carried edit applied). Never fails: the file is written, so what goes wrong
/// here becomes a note.
fn after_write(
    analyzer: &mut Analyzer,
    output: &Path,
    settings: &ProcessSettings,
    carry: &Carry<'_>,
    duration: Seconds,
) -> ProcessDone {
    let mut notes = Vec::new();
    let edit_carried = match (&settings.edits, carry.saved) {
        (Some(store), Some(saved)) if carry.edit.edited || carry.edit.confirmed => {
            match carry_to(store, saved, carry, output, &analyzer.settings) {
                Ok(()) => true,
                Err(e) => {
                    notes.push(format!("the grid edit was not carried over: {e}"));
                    false
                }
            }
        }
        _ => false,
    };
    let (analysis, edit) = match analyzer.analyze_fresh(output, &mut Timings::default(), None) {
        Ok(report) => {
            let mut record = report.record;
            let edit = settings
                .edits
                .as_ref()
                .map(|store| apply_saved(&mut record, store))
                .unwrap_or_default();
            if edit_carried && carry.edit.confirmed && !edit.confirmed {
                notes.push(
                    "the confirmed grid did not come back from the written file's analysis; \
                     confirm it again"
                        .to_owned(),
                );
            }
            // The written file's own analysis: where its grid is compared with the one exported.
            (Some(Box::new(record)), edit)
        }
        Err(Error::Cancelled) => {
            notes.push(
                "cancelled before the written file was analysed; it is analysed when next opened"
                    .to_owned(),
            );
            (None, EditState::default())
        }
        Err(e) => {
            notes.push(format!("the written file could not be analysed: {e}"));
            (None, EditState::default())
        }
    };
    if !notes.is_empty() {
        tracing::warn!(path = %output.display(), notes = ?notes, "after the write");
    }
    ProcessDone {
        output: output.to_path_buf(),
        duration,
        analysis,
        edit,
        edit_carried,
        notes,
    }
}

/// Carries the edit over to `output`, made on the output's audio, which a loudness pass
/// measures (quick, and not cancelled with the batch: the edit is user work and the file is
/// already written).
fn carry_to(
    store: &EditStore,
    saved: &SavedEdit,
    carry: &Carry<'_>,
    output: &Path,
    analysis: &AnalysisSettings,
) -> Result<()> {
    let loudness_only = AnalysisSettings {
        grid: false,
        ..analysis.clone()
    };
    let measured = Analyzer::load(loudness_only, None, CancelToken::new())?.analyze(output)?;
    carry_edit(
        store,
        saved,
        carry.grid,
        carry.edit.confirmed,
        carry.trim,
        &measured.record.path,
        audio_of(&measured.record),
    )
}
