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
    ExportRow, FileEntry, FileInfo, IpcError, JobEvent, JobId, Replan, RowAnalysis, RowPlan,
    RowUpdate, SessionRow, SessionSnapshot,
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
        })
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
                })
                .collect(),
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
    /// analysed. No export of it stands any more. Returns the row and its plan, when analysed.
    pub fn set_reverted(
        &mut self,
        file_id: u32,
        info: FileInfo,
        analysis: Option<(AnalysisRecord, EditState)>,
    ) -> Option<RowUpdate> {
        let revision = self.revision;
        let settings = self.settings;
        let t = self.files.get_mut(&file_id)?;
        t.entry.info = info;
        t.export = None;
        t.export_running = None;
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
                let t = self.files.get_mut(&file_id)?;
                let row = export_row::planned(&plan, t.export.as_deref());
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
                let t = self.files.get_mut(&file_id)?;
                let row = export_row::skipped(&outcome, t.export.as_deref());
                t.export_running = None;
                t.export = Some(row.clone());
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
        let update = if done.output == Path::new(&t.entry.path) {
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
        if let Some(ipc) = lock().on_engine_event(job_id, event) {
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
