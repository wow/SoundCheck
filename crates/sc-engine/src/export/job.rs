//! One export as the app runs it: the batch's events turned into what the UI receives, the
//! session's rows kept current, and, after the last file, the batch's rekordbox XML and grid
//! report. The batch itself is [`run_batch`], exactly as `sc-cli process` runs it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Instant, SystemTime};

use sc_core::Error;
use sc_core::analysis::AnalysisSettings;
use sc_core::export::{BatchMode, ExportPlan};
use sc_core::ipc::{ExportArtefacts, ExportSummary, ExportedWith, IpcError, JobEvent, JobId};
use sc_io::cache::Cache;
use sc_io::txn::TxnReport;

use super::artefacts::{Artefacts, BatchRow, RowAction, XmlSelect, write_artefacts};
use super::rows::{failed_row, not_written_row, written_row};
use super::{ProcessDone, ProcessSettings};
use crate::batch::{BatchFile, BatchSettings, EngineEvent, Task, run_batch};
use crate::cancel::CancelToken;
use crate::session::Session;

/// How the app runs one export.
#[derive(Debug, Clone)]
pub struct ExportJob {
    /// Analysis settings, shared by every file.
    pub analysis: AnalysisSettings,
    /// Worker threads.
    pub workers: usize,
    /// The analysis cache.
    pub cache: Option<Cache>,
    /// How the files are exported.
    pub process: ProcessSettings,
    /// Where an export in place makes the folder for its artefacts; `None` when the system
    /// names no such place.
    pub exports_root: Option<PathBuf>,
}

/// The rows of one batch, in the order of its files, as its events say what happened.
struct Rows<'a> {
    files: &'a [BatchFile],
    mode: BatchMode,
    index: BTreeMap<u32, usize>,
    plans: BTreeMap<u32, Box<ExportPlan>>,
    reports: BTreeMap<u32, Box<TxnReport>>,
    found: Vec<Option<BatchRow>>,
    /// Files cancelled, which their rows count among the failed.
    cancelled: u32,
}

impl<'a> Rows<'a> {
    fn new(files: &'a [BatchFile], mode: BatchMode) -> Self {
        Self {
            files,
            mode,
            index: files
                .iter()
                .enumerate()
                .map(|(i, f)| (f.file_id, i))
                .collect(),
            plans: BTreeMap::new(),
            reports: BTreeMap::new(),
            found: files.iter().map(|_| None).collect(),
            cancelled: 0,
        }
    }

    fn set(&mut self, file_id: u32, row: impl FnOnce(&Path, BatchMode) -> BatchRow) {
        if let Some(&i) = self.index.get(&file_id) {
            self.found[i] = Some(row(&self.files[i].path, self.mode));
        }
    }

    fn see(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::Processing { file_id, plan } => {
                self.plans.insert(*file_id, plan.clone());
            }
            EngineEvent::Written { file_id, report } => {
                self.reports.insert(*file_id, report.clone());
            }
            EngineEvent::Done { file_id, done } => self.see_done(*file_id, done),
            EngineEvent::ExportSkipped {
                file_id,
                outcome,
                duration,
                codec,
                xml,
            } => self.set(*file_id, |file, mode| {
                not_written_row(file, mode, outcome, *duration, *codec, xml)
            }),
            EngineEvent::Failed { file_id, error } => {
                self.set(*file_id, |file, mode| failed_row(file, Some(mode), error));
            }
            EngineEvent::Cancelled { file_id } => {
                self.cancelled += 1;
                self.set(*file_id, |file, mode| {
                    failed_row(file, Some(mode), &Error::Cancelled)
                });
            }
            EngineEvent::Started { .. }
            | EngineEvent::Progress { .. }
            | EngineEvent::Analysed { .. }
            | EngineEvent::Batch(_) => {}
        }
    }

    fn see_done(&mut self, file_id: u32, done: &ProcessDone) {
        let plan = self.plans.remove(&file_id);
        let report = self.reports.remove(&file_id);
        self.set(file_id, |file, mode| match (&plan, &report) {
            (Some(plan), Some(report)) => written_row(file, mode, plan, report, done),
            _ => failed_row(
                file,
                Some(mode),
                &Error::Internal("a written file lost its report".into()),
            ),
        });
    }

    fn finish(self) -> (Vec<BatchRow>, u32) {
        (self.found.into_iter().flatten().collect(), self.cancelled)
    }
}

/// What became of a batch, for a window that reloads.
fn summary(
    job_id: JobId,
    rows: &[BatchRow],
    not_failed: u32,
    was_cancelled: bool,
    started: Instant,
) -> ExportSummary {
    let count = |action| {
        u32::try_from(rows.iter().filter(|r| r.action == action).count()).unwrap_or(u32::MAX)
    };
    ExportSummary {
        job_id,
        written: count(RowAction::Written),
        xml_only: count(RowAction::XmlOnly),
        skipped: count(RowAction::Skipped),
        failed: count(RowAction::Failed).saturating_sub(not_failed),
        cancelled: was_cancelled,
        seconds: started.elapsed().as_secs_f64(),
    }
}

/// Runs one export over `file_ids`, sending its events in order and ending with `finished`.
///
/// The session is locked only while an event is turned into its IPC form. A file written in place
/// is probed again first, outside the lock. With the XML on, the batch's artefacts are written
/// after its last file (also after a cancel, for the files written until then), unless every file
/// failed: into the export's folder, or for an export in place into a new folder under
/// [`ExportJob::exports_root`].
pub fn run_export_job(
    session: &Mutex<Session>,
    job_id: JobId,
    file_ids: &[u32],
    job: &ExportJob,
    cancel: &CancelToken,
    send: &mut dyn FnMut(JobEvent),
) {
    let lock = || session.lock().unwrap_or_else(PoisonError::into_inner);
    let started = Instant::now();
    let with = ExportedWith::new(
        job.process.decide,
        job.process.export,
        job.process.out_dir.as_deref(),
    );
    {
        let mut session = lock();
        session.set_artefacts(None);
        session.set_last_export(None);
        session.begin_export(job_id, with);
    }
    let files = lock().batch(file_ids);
    let settings = BatchSettings {
        analysis: job.analysis.clone(),
        workers: job.workers,
        cache: job.cache.clone(),
        task: Task::Process(Box::new(job.process.clone())),
    };
    let mode = job.process.export.batch_mode;
    let mut rows = Rows::new(&files, mode);
    let result = run_batch(&files, &settings, cancel, &mut |event| {
        rows.see(&event);
        // What the file now is, read before the session is locked.
        let probed = match &event {
            EngineEvent::Done { file_id, done } if job.process.out_dir.is_none() => {
                Some((*file_id, sc_io::probe(&done.output)))
            }
            _ => None,
        };
        let mut session = lock();
        if let Some((file_id, info)) = probed {
            session.set_info(file_id, info);
        }
        let ipc = session.on_engine_event(job_id, event);
        drop(session);
        if let Some(ipc) = ipc {
            send(ipc);
        }
    });
    match result {
        Ok(summary_of_batch) => {
            let (rows, cancelled) = rows.finish();
            if let Some(event) = write_batch_artefacts(job_id, job, &rows) {
                match &event {
                    JobEvent::Artefacts { artefacts, .. } => {
                        lock().set_artefacts(Some(Ok(artefacts.clone())));
                    }
                    JobEvent::ArtefactsFailed { error, .. } => {
                        lock().set_artefacts(Some(Err(error.clone())));
                    }
                    _ => {}
                }
                send(event);
            }
            lock().set_last_export(Some(summary(
                job_id,
                &rows,
                cancelled,
                summary_of_batch.was_cancelled,
                started,
            )));
            lock().end_export(job_id);
            send(JobEvent::Finished {
                job_id,
                cancelled: summary_of_batch.was_cancelled,
            });
        }
        Err(error) => {
            lock().end_export(job_id);
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

/// Writes the batch's artefacts, when it has any to write; the event saying how that went.
fn write_batch_artefacts(job_id: JobId, job: &ExportJob, rows: &[BatchRow]) -> Option<JobEvent> {
    let process = &job.process;
    if !process.export.xml || rows.iter().all(|r| r.action == RowAction::Failed) {
        return None;
    }
    let now = SystemTime::now();
    let playlist = sc_io::rekordbox::playlist_name(now);
    let written = artefacts_folder(process.out_dir.as_deref(), job.exports_root.as_deref(), now)
        .and_then(|dir| write_artefacts(&dir, rows, XmlSelect::Batch, &playlist).map(|a| (dir, a)));
    Some(match written {
        Ok((dir, artefacts)) => {
            tracing::info!(
                job_id,
                folder = %dir.display(),
                listed = artefacts.listed,
                "artefacts written"
            );
            JobEvent::Artefacts {
                job_id,
                artefacts: export_artefacts(&dir, &artefacts),
            }
        }
        Err(error) => {
            tracing::warn!(job_id, error = %error, "artefacts not written");
            JobEvent::ArtefactsFailed {
                job_id,
                error: IpcError::from(error),
            }
        }
    })
}

/// The IPC form of `artefacts`, written in `dir`.
#[must_use]
pub fn export_artefacts(dir: &Path, artefacts: &Artefacts) -> ExportArtefacts {
    ExportArtefacts {
        folder: dir.display().to_string(),
        xml: artefacts.xml.as_ref().map(|p| p.display().to_string()),
        report: Some(artefacts.report.display().to_string()),
        listed: u32::try_from(artefacts.listed).unwrap_or(u32::MAX),
    }
}

/// A new folder for one batch's artefacts under `root`.
///
/// # Errors
/// [`Error::Internal`] without a root; what [`sc_io::artefacts::new_batch_folder`] returns.
pub fn new_artefacts_folder(root: Option<&Path>, now: SystemTime) -> sc_core::Result<PathBuf> {
    let root = root.ok_or_else(|| Error::Internal("no folder to keep exports in".to_owned()))?;
    sc_io::artefacts::new_batch_folder(root, now)
}

/// Where a batch's artefacts go: its export folder (created if no copy was written, never on a
/// rekordbox USB export), else a new folder under `root`.
fn artefacts_folder(
    out: Option<&Path>,
    root: Option<&Path>,
    now: SystemTime,
) -> sc_core::Result<PathBuf> {
    match out {
        Some(dir) => {
            sc_io::txn::check_folder_not_rekordbox(dir, &sc_io::txn::SystemVolumes)?;
            std::fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.to_path_buf(),
                source,
            })?;
            Ok(dir.to_path_buf())
        }
        None => new_artefacts_folder(root, now),
    }
}
