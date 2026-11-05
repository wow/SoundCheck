//! The desktop app's state between commands: the files the user added (each with a stable id),
//! the latest analysis of each, and the decide settings. Keeping it here, next to the pipeline,
//! leaves the Tauri shell a thin wrapper and makes every IPC event testable without a window.
//!
//! Records are kept without their grid evidence (the model beats, activations and onsets, about
//! 150 KB a track); the disk cache holds it for the grid view. A saved grid edit replaces the
//! analysed grid before the evidence is dropped, and a confirmed grid needs no review.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use sc_core::analysis::{AnalysisRecord, Grid};
use sc_core::ipc::{
    ExportArtefacts, ExportRow, ExportSummary, ExportedWith, FileEntry, FileInfo, IpcError,
    JobEvent, JobId, Replan, RowAnalysis, RowPlan, RowUpdate, SessionRow, SessionSnapshot,
};
use sc_core::plan::{DecideSettings, GainPlan, Plan};
use sc_core::{Lufs, Result};
use sc_io::edits::EditStore;
use unicode_normalization::UnicodeNormalization;

use crate::analyze::CacheStatus;
use crate::batch::{BatchFile, BatchSettings, EngineEvent, Task, run_batch};
use crate::cancel::CancelToken;
use crate::decide::decide;
use crate::edits::{EditState, apply_saved};
use crate::export::ProcessDone;
use crate::export::row as export_row;

/// One file the user added.
#[derive(Debug, Clone)]
struct Tracked {
    entry: FileEntry,
    record: Option<AnalysisRecord>,
    edit: EditState,
    /// The latest export that ended, written or not.
    export: Option<Box<ExportRow>>,
    /// The export running now: a failure or a cancel leaves [`Self::export`] as it was.
    export_running: Option<Box<ExportRow>>,
    /// A change of the file made in place is in effect: `revert_file` can put it back.
    revertible: bool,
    /// A grid different from what the standing export wrote was saved since.
    export_stale: bool,
    /// The copy the standing export wrote to a folder.
    copy: Option<PathBuf>,
    /// What the file was when `record` was analysed (`sc_io::txn::file_identity`).
    identity: Option<String>,
}

impl Tracked {
    fn plan(&self, settings: &DecideSettings) -> Option<Plan> {
        let record = self.record.as_ref()?;
        Some(decide(
            record,
            self.entry.info.codec,
            settings,
            self.edit.confirmed,
        ))
    }

    fn row(&self, cached: bool) -> Option<RowAnalysis> {
        Some(row_of(self.record.as_ref()?, self.edit, cached))
    }

    /// The row and its plan, when analysed; never served from the cache (a file just changed).
    fn update(&self, file_id: u32, settings: &DecideSettings, revision: u32) -> Option<RowUpdate> {
        Some(RowUpdate {
            file_id,
            row: Box::new(self.row(false)?),
            plan: self.plan(settings)?,
            revision,
            export_stale: self.export_stale,
        })
    }

    /// Notes what the file is now: its record describes that.
    fn note_identity(&mut self) {
        self.identity = sc_io::txn::file_identity(Path::new(&self.entry.path)).ok();
    }
}

/// The table row of `record`, whose grid carries `edit`.
fn row_of(record: &AnalysisRecord, edit: EditState, cached: bool) -> RowAnalysis {
    RowAnalysis {
        edited: edit.edited,
        confirmed: edit.confirmed,
        ..RowAnalysis::from_record(record, cached)
    }
}

/// What planning one file's export takes.
#[derive(Debug, Clone)]
pub struct PlanningRow {
    /// Its analysis with the grid as shown.
    pub record: AnalysisRecord,
    /// The state of that grid.
    pub edit: EditState,
    /// An earlier change of it is in effect.
    pub revertible: bool,
    /// What the file was when that analysis was made.
    pub identity: Option<String>,
}

/// Files, analyses and settings of one app session.
#[derive(Debug)]
pub struct Session {
    next_file: u32,
    next_job: JobId,
    files: BTreeMap<u32, Tracked>,
    by_path: HashMap<String, u32>,
    settings: DecideSettings,
    /// Increases with every settings change, so the UI keeps the newest plan of a row whose
    /// analysis and a replan crossed on the way.
    revision: u32,
    /// Where the user's grid edits are saved; `None` applies none.
    edits: Option<EditStore>,
    /// The latest batch's rekordbox XML and grid report, or why they could not be written, for a
    /// window that reloads and never saw the event.
    artefacts: Option<std::result::Result<ExportArtefacts, IpcError>>,
    /// What became of the latest batch.
    last_export: Option<ExportSummary>,
    /// What each running export is made with.
    export_jobs: BTreeMap<JobId, ExportedWith>,
}

impl Session {
    /// An empty session deciding with `settings`.
    #[must_use]
    pub fn new(settings: DecideSettings) -> Self {
        Self {
            next_file: 1,
            next_job: 1,
            files: BTreeMap::new(),
            by_path: HashMap::new(),
            settings,
            revision: 0,
            edits: None,
            artefacts: None,
            last_export: None,
            export_jobs: BTreeMap::new(),
        }
    }

    /// The session whose jobs apply the grid edits saved in `store` to every analysis.
    #[must_use]
    pub fn with_edits(mut self, store: EditStore) -> Self {
        self.edits = Some(store);
        self
    }

    /// Adds probed files, in order, skipping any already in the session (by NFC path); returns
    /// the new entries with their ids.
    pub fn add(&mut self, files: Vec<(PathBuf, FileInfo)>) -> Vec<FileEntry> {
        let mut added = Vec::new();
        for (path, info) in files {
            let key: String = path.to_string_lossy().nfc().collect();
            if self.by_path.contains_key(&key) {
                continue;
            }
            let file_id = self.next_file;
            self.next_file += 1;
            let entry = FileEntry {
                file_id,
                path: key.clone(),
                info,
            };
            self.by_path.insert(key, file_id);
            self.files.insert(
                file_id,
                Tracked {
                    entry: entry.clone(),
                    record: None,
                    edit: EditState::default(),
                    export: None,
                    export_running: None,
                    revertible: false,
                    export_stale: false,
                    copy: None,
                    identity: None,
                },
            );
            added.push(entry);
        }
        added
    }

    /// Forgets every file and its analysis (the disk cache keeps the analyses). Ids are never
    /// reused, so events still in flight from a cancelled job cannot land on a new row.
    pub fn clear(&mut self) {
        self.files.clear();
        self.by_path.clear();
        self.artefacts = None;
        self.last_export = None;
    }

    /// What became of the latest batch; `None` while one runs, and when it could not start.
    pub fn set_last_export(&mut self, summary: Option<ExportSummary>) {
        self.last_export = summary;
    }

    /// An export starts as job `job_id`, made with `with`: its rows say so.
    pub fn begin_export(&mut self, job_id: JobId, with: ExportedWith) {
        self.export_jobs.insert(job_id, with);
    }

    /// The export `job_id` has ended.
    pub fn end_export(&mut self, job_id: JobId) {
        self.export_jobs.remove(&job_id);
    }

    /// The settings revision now.
    #[must_use]
    pub fn revision(&self) -> u32 {
        self.revision
    }

    /// The copy the file's standing export wrote to a folder.
    #[must_use]
    pub fn exported_copy(&self, file_id: u32) -> Option<PathBuf> {
        self.files.get(&file_id)?.copy.clone()
    }

    /// The latest batch's artefacts, or why they could not be written; `None` for a batch that
    /// has none (yet).
    pub fn set_artefacts(
        &mut self,
        artefacts: Option<std::result::Result<ExportArtefacts, IpcError>>,
    ) {
        self.artefacts = artefacts;
    }

    /// What planning the file's export takes. `None` until it is analysed.
    #[must_use]
    pub fn planning_row(&self, file_id: u32) -> Option<PlanningRow> {
        let t = self.files.get(&file_id)?;
        Some(PlanningRow {
            record: t.record.clone()?,
            edit: t.edit,
            revertible: t.revertible,
            identity: t.identity.clone(),
        })
    }

    /// Files in the session.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no file was added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The batch for `file_ids`, in that order; unknown ids are left out.
    #[must_use]
    pub fn batch(&self, file_ids: &[u32]) -> Vec<BatchFile> {
        file_ids
            .iter()
            .filter_map(|id| self.files.get(id))
            .map(|t| BatchFile {
                file_id: t.entry.file_id,
                path: PathBuf::from(&t.entry.path),
                duration_hint: t.entry.info.duration.map(|d| d.0),
            })
            .collect()
    }

    /// A new job id.
    pub fn next_job_id(&mut self) -> JobId {
        let id = self.next_job;
        self.next_job += 1;
        id
    }

    /// The settings rows are decided with.
    #[must_use]
    pub fn settings(&self) -> DecideSettings {
        self.settings
    }

    /// Changes the settings and returns every analysed row's new plan under a new revision.
    ///
    /// # Errors
    /// [`sc_core::Error::InvalidArgument`] when a value is outside its limits; the settings stay
    /// as they were.
    pub fn set_settings(&mut self, settings: DecideSettings) -> Result<Replan> {
        settings.validate()?;
        self.settings = settings;
        self.revision += 1;
        let plans = self
            .files
            .values()
            .filter_map(|t| {
                t.plan(&settings).map(|plan| RowPlan {
                    file_id: t.entry.file_id,
                    plan,
                })
            })
            .collect();
        Ok(Replan {
            revision: self.revision,
            plans,
        })
    }

    /// Every row with its analysis and plan, for a window that reloads.
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            revision: self.revision,
            rows: self
                .files
                .values()
                .map(|t| SessionRow {
                    entry: t.entry.clone(),
                    row: t.row(true).map(Box::new),
                    plan: t.plan(&self.settings),
                    export: t.export_running.clone().or_else(|| t.export.clone()),
                    revertible: t.revertible,
                    export_stale: t.export_stale,
                })
                .collect(),
            running_jobs: Vec::new(),
            last_export: self.last_export.clone(),
            artefacts: self
                .artefacts
                .as_ref()
                .and_then(|a| a.as_ref().ok())
                .cloned(),
            artefacts_failed: self
                .artefacts
                .as_ref()
                .and_then(|a| a.as_ref().err())
                .cloned(),
        }
    }

    /// The file with `file_id`.
    #[must_use]
    pub fn entry(&self, file_id: u32) -> Option<FileEntry> {
        self.files.get(&file_id).map(|t| t.entry.clone())
    }

    /// The gain processing would apply to the file, in dB: 0 when it is skipped or at target.
    #[must_use]
    pub fn planned_gain(&self, file_id: u32) -> Option<f64> {
        let plan = self.plan(file_id)?;
        Some(match plan.gain {
            Some(GainPlan::Gain { gain_db, .. } | GainPlan::GlobalGain { gain_db, .. }) => gain_db,
            Some(GainPlan::AtTarget) | None => 0.0,
        })
    }

    /// Replaces the analysed file's grid with `grid` (a saved edit's) and its edit state;
    /// returns the row and its plan under the current settings, or `None` for a file without
    /// an analysis.
    pub fn set_grid(
        &mut self,
        file_id: u32,
        grid: Option<Grid>,
        edit: EditState,
    ) -> Option<RowUpdate> {
        let t = self.files.get_mut(&file_id)?;
        let record = t.record.as_mut()?;
        // What the export wrote no longer holds: another grid, or one withheld now confirmed.
        let released = t.export.as_ref().is_some_and(|e| e.grid_withheld) && edit.confirmed;
        if t.export.is_some() && (record.grid != grid || released) {
            t.export_stale = true;
        }
        if grid.is_some() {
            record.grid_skipped = None;
        }
        record.grid = grid;
        t.edit = edit;
        let row = Box::new(t.row(true)?);
        let plan = t.plan(&self.settings)?;
        Some(RowUpdate {
            file_id,
            row,
            plan,
            revision: self.revision,
            export_stale: t.export_stale,
        })
    }

    /// What the file's headers now say, after a change of the file.
    pub fn set_info(&mut self, file_id: u32, info: FileInfo) {
        if let Some(t) = self.files.get_mut(&file_id) {
            t.entry.info = info;
        }
    }

    /// The file `file_id` is its original again (a revert): `info` is what its headers say and
    /// `analysis` its own analysis with its saved edit applied, or `None` when it could not be
    /// analysed. No export of it stands any more; with `earlier_changes` still in effect, another
    /// revert can go further back. Returns the row and its plan, when analysed.
    pub fn set_reverted(
        &mut self,
        file_id: u32,
        info: FileInfo,
        analysis: Option<(AnalysisRecord, EditState)>,
        earlier_changes: usize,
    ) -> Option<RowUpdate> {
        let revision = self.revision;
        let settings = self.settings;
        let t = self.files.get_mut(&file_id)?;
        t.entry.info = info;
        t.export = None;
        t.export_running = None;
        t.revertible = earlier_changes > 0;
        t.export_stale = false;
        t.copy = None;
        t.note_identity();
        let (record, edit) = analysis.unzip();
        t.record = record.map(|mut record| {
            record.evidence = None;
            record
        });
        t.edit = edit.unwrap_or_default();
        t.update(file_id, &settings, revision)
    }

    /// The plan of one analysed file under the current settings.
    #[must_use]
    pub fn plan(&self, file_id: u32) -> Option<Plan> {
        self.files.get(&file_id)?.plan(&self.settings)
    }

    /// The median S-P95 of the analysed rows: what "Calibrate from my library" sets the DJ target
    /// to. `None` when no row has one.
    #[must_use]
    pub fn median_short_term_p95(&self) -> Option<Lufs> {
        let mut values: Vec<f64> = self
            .files
            .values()
            .filter_map(|t| t.record.as_ref()?.loudness.short_term_p95.map(|l| l.0))
            .collect();
        if values.is_empty() {
            return None;
        }
        values.sort_by(f64::total_cmp);
        let mid = values.len() / 2;
        let median = if values.len() % 2 == 1 {
            values[mid]
        } else {
            f64::midpoint(values[mid - 1], values[mid])
        };
        Some(Lufs(median))
    }

    /// Turns an engine event of a job into what the UI receives, storing analyses and export
    /// rows on the way; the evidence of an analysis stays out of both. `None` for an export event
    /// of a file no longer in the session (the list was cleared meanwhile).
    pub fn on_engine_event(&mut self, job_id: JobId, event: EngineEvent) -> Option<JobEvent> {
        Some(match event {
            EngineEvent::Started { file_id } => JobEvent::Started { job_id, file_id },
            EngineEvent::Progress { file_id, fraction } => JobEvent::Progress {
                job_id,
                file_id,
                fraction,
            },
            EngineEvent::Analysed {
                file_id,
                report,
                edit,
            } => {
                let mut record = report.record;
                record.evidence = None;
                let codec = self
                    .files
                    .get(&file_id)
                    .map_or(sc_core::plan::Codec::Other, |t| t.entry.info.codec);
                let row = Box::new(row_of(&record, edit, report.cache == CacheStatus::Hit));
                let plan = decide(&record, codec, &self.settings, edit.confirmed);
                if let Some(t) = self.files.get_mut(&file_id) {
                    t.record = Some(record);
                    t.edit = edit;
                    t.note_identity();
                }
                JobEvent::Analysed {
                    job_id,
                    file_id,
                    row,
                    plan,
                    revision: self.revision,
                }
            }
            EngineEvent::Failed { file_id, error } => {
                self.end_running_export(file_id);
                failed_event(job_id, file_id, error)
            }
            EngineEvent::Cancelled { file_id } => {
                self.end_running_export(file_id);
                JobEvent::Cancelled { job_id, file_id }
            }
            EngineEvent::Batch(p) => JobEvent::Batch {
                job_id,
                done: saturating_u32(p.done),
                total: saturating_u32(p.total),
                eta_ms: p.eta.map(millis),
                // Display precision.
                #[allow(clippy::cast_possible_truncation)]
                realtime_x: p.realtime_x.map(|x| x as f32),
            },
            EngineEvent::Processing { file_id, plan } => {
                let with = self.export_jobs.get(&job_id).cloned();
                let t = self.files.get_mut(&file_id)?;
                let mut row = export_row::planned(&plan, t.revertible);
                row.exported_with = with;
                t.export_running = Some(row.clone());
                JobEvent::Processing {
                    job_id,
                    file_id,
                    row,
                }
            }
            EngineEvent::Written { file_id, report } => {
                let t = self.files.get_mut(&file_id)?;
                let Some(row) = t.export_running.take() else {
                    return Some(lost_report(job_id, file_id));
                };
                let row = export_row::written(row, &report);
                t.revertible = row.revertible;
                t.export_running = Some(row.clone());
                JobEvent::Written {
                    job_id,
                    file_id,
                    row,
                }
            }
            EngineEvent::ExportSkipped {
                file_id, outcome, ..
            } => {
                let with = self.export_jobs.get(&job_id).cloned();
                let t = self.files.get_mut(&file_id)?;
                let mut row = export_row::skipped(&outcome, t.revertible);
                row.exported_with = with;
                t.export_running = None;
                t.export = Some(row.clone());
                t.export_stale = false;
                JobEvent::ExportSkipped {
                    job_id,
                    file_id,
                    row,
                }
            }
            EngineEvent::Done { file_id, done } => return self.on_done(job_id, file_id, *done),
        })
    }

    /// A file's export ended without changing it: its last export stands.
    fn end_running_export(&mut self, file_id: u32) {
        if let Some(t) = self.files.get_mut(&file_id) {
            t.export_running = None;
        }
    }

    /// A file is written and checked. Written in place, the row now is the written file: its own
    /// analysis (none when that did not finish: what was measured before no longer holds).
    fn on_done(&mut self, job_id: JobId, file_id: u32, mut done: ProcessDone) -> Option<JobEvent> {
        let revision = self.revision;
        let settings = self.settings;
        let t = self.files.get_mut(&file_id)?;
        let Some(row) = t.export_running.take() else {
            return Some(lost_report(job_id, file_id));
        };
        let row = export_row::done(row, &done);
        t.export = Some(row.clone());
        t.export_stale = false;
        let in_place = done.output == Path::new(&t.entry.path);
        t.copy = (!in_place).then(|| done.output.clone());
        let update = if in_place {
            t.note_identity();
            t.record = done.analysis.take().map(|mut record| {
                record.evidence = None;
                *record
            });
            t.edit = if t.record.is_some() {
                done.edit
            } else {
                EditState::default()
            };
            t.update(file_id, &settings, revision).map(Box::new)
        } else {
            None
        };
        Some(JobEvent::ExportDone {
            job_id,
            file_id,
            row,
            in_place,
            update,
        })
    }
}

/// The failed event of `file_id`.
fn failed_event(job_id: JobId, file_id: u32, error: sc_core::Error) -> JobEvent {
    JobEvent::Failed {
        job_id,
        file_id,
        error: IpcError {
            file_id: Some(file_id),
            ..IpcError::from(error)
        },
    }
}

/// An export event that came without the ones before it, which a batch never sends.
fn lost_report(job_id: JobId, file_id: u32) -> JobEvent {
    failed_event(
        job_id,
        file_id,
        sc_core::Error::Internal("a written file lost its report".into()),
    )
}

/// Runs one analysis job over `file_ids` (whatever `settings.task` says; an export runs through
/// [`crate::export::run_export_job`]), sending its events in
/// order and ending with `finished`.
///
/// The session is locked only while an event is turned into its IPC form, never while a file is
/// analysed, so the UI can add files or change settings during a job.
pub fn run_job(
    session: &Mutex<Session>,
    job_id: JobId,
    file_ids: &[u32],
    settings: &BatchSettings,
    cancel: &CancelToken,
    send: &mut dyn FnMut(JobEvent),
) {
    let lock = || session.lock().unwrap_or_else(PoisonError::into_inner);
    let (files, edits) = {
        let s = lock();
        (s.batch(file_ids), s.edits.clone())
    };
    let settings = BatchSettings {
        task: Task::Analyze,
        ..settings.clone()
    };
    let result = run_batch(&files, &settings, cancel, &mut |mut event| {
        // The saved edit is read and refitted before the session is locked.
        if let (EngineEvent::Analysed { report, edit, .. }, Some(store)) = (&mut event, &edits) {
            *edit = apply_saved(&mut report.record, store);
        }
        // Not sent under the lock: a slow receiver would hold up every command that needs it.
        let ipc = lock().on_engine_event(job_id, event);
        if let Some(ipc) = ipc {
            send(ipc);
        }
    });
    match result {
        Ok(summary) => send(JobEvent::Finished {
            job_id,
            cancelled: summary.was_cancelled,
        }),
        Err(error) => {
            send(JobEvent::Aborted {
                job_id,
                error: IpcError::from(error),
            });
            send(JobEvent::Finished {
                job_id,
                cancelled: false,
            });
        }
    }
}

fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}
