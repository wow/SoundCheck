//! The desktop app's state between commands: the files the user added (each with a stable id),
//! the latest analysis of each, and the decide settings. Keeping it here, next to the pipeline,
//! leaves the Tauri shell a thin wrapper and makes every IPC event testable without a window.
//!
//! Records are kept without their grid evidence (the model beats and activations, about 100 KB a
//! track); the disk cache holds it for the grid view.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use sc_core::analysis::AnalysisRecord;
use sc_core::ipc::{
    FileEntry, FileInfo, IpcError, JobEvent, JobId, Replan, RowAnalysis, RowPlan, SessionRow,
    SessionSnapshot,
};
use sc_core::plan::{DecideSettings, Plan};
use sc_core::{Lufs, Result};
use unicode_normalization::UnicodeNormalization;

use crate::analyze::CacheStatus;
use crate::batch::{BatchFile, BatchSettings, EngineEvent, run_batch};
use crate::cancel::CancelToken;
use crate::decide::decide;

/// One file the user added.
#[derive(Debug, Clone)]
struct Tracked {
    entry: FileEntry,
    record: Option<AnalysisRecord>,
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
        }
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
                t.record.as_ref().map(|record| RowPlan {
                    file_id: t.entry.file_id,
                    plan: decide(record, t.entry.info.codec, &settings),
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
                    row: t
                        .record
                        .as_ref()
                        .map(|r| Box::new(RowAnalysis::from_record(r, true))),
                    plan: t
                        .record
                        .as_ref()
                        .map(|r| decide(r, t.entry.info.codec, &self.settings)),
                })
                .collect(),
        }
    }

    /// The plan of one analysed file under the current settings.
    #[must_use]
    pub fn plan(&self, file_id: u32) -> Option<Plan> {
        let t = self.files.get(&file_id)?;
        let record = t.record.as_ref()?;
        Some(decide(record, t.entry.info.codec, &self.settings))
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

    /// Turns an engine event into what the UI receives, storing analyses on the way.
    pub fn on_engine_event(&mut self, job_id: JobId, event: EngineEvent) -> JobEvent {
        match event {
            EngineEvent::Started { file_id } => JobEvent::Started { job_id, file_id },
            EngineEvent::Progress { file_id, fraction } => JobEvent::Progress {
                job_id,
                file_id,
                fraction,
            },
            EngineEvent::Analysed { file_id, report } => {
                let mut record = report.record;
                record.evidence = None;
                let row = Box::new(RowAnalysis::from_record(
                    &record,
                    report.cache == CacheStatus::Hit,
                ));
                let codec = self
                    .files
                    .get(&file_id)
                    .map_or(sc_core::plan::Codec::Other, |t| t.entry.info.codec);
                let plan = decide(&record, codec, &self.settings);
                if let Some(t) = self.files.get_mut(&file_id) {
                    t.record = Some(record);
                }
                JobEvent::Analysed {
                    job_id,
                    file_id,
                    row,
                    plan,
                    revision: self.revision,
                }
            }
            EngineEvent::Failed { file_id, error } => JobEvent::Failed {
                job_id,
                file_id,
                error: IpcError {
                    file_id: Some(file_id),
                    ..IpcError::from(error)
                },
            },
            EngineEvent::Cancelled { file_id } => JobEvent::Cancelled { job_id, file_id },
            EngineEvent::Batch(p) => JobEvent::Batch {
                job_id,
                done: saturating_u32(p.done),
                total: saturating_u32(p.total),
                eta_ms: p.eta.map(millis),
                // Display precision.
                #[allow(clippy::cast_possible_truncation)]
                realtime_x: p.realtime_x.map(|x| x as f32),
            },
        }
    }
}

/// Runs one job over `file_ids`, sending its events in order and ending with `finished`.
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
    let files = lock().batch(file_ids);
    let result = run_batch(&files, settings, cancel, &mut |event| {
        let ipc = lock().on_engine_event(job_id, event);
        send(ipc);
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
