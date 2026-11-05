//! What an engine event of a job does to the session, and what the UI receives for it.

use std::path::Path;
use std::time::Duration;

use sc_core::ipc::{IpcError, JobEvent, JobId};

use super::{Session, row_of};
use crate::analyze::CacheStatus;
use crate::batch::EngineEvent;
use crate::decide::decide;
use crate::edits::EditState;
use crate::export::ProcessDone;
use crate::export::row as export_row;

impl Session {
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
                let identity = report.identity;
                let bpm_range = report.bpm_range;
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
                    // The file changed since what stands was made of it.
                    if t.export.is_some() && t.identity.is_some() && identity != t.identity {
                        t.export_stale = true;
                    }
                    t.identity = identity;
                    t.analysed_range = Some(bpm_range);
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
            let (identity, bpm_range) = done.analysed.take().unzip();
            t.identity = identity;
            t.analysed_range = bpm_range;
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

fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}
