//! The export commands' bodies: starting an export, reverting a file, writing a rekordbox XML of
//! analysed rows and forgetting a change recovery left pending. Every one of them is refused while
//! the start-up recovery runs: the engine would only wait for it, and the user is better told.

use std::path::{Path, PathBuf};

use sc_core::Error;
use sc_core::analysis::AnalysisRecord;
use sc_core::export::Place as ExportPlace;
use sc_core::ipc::{
    ExportArtefacts, ExportRequest, ForgetResult, IpcError, IpcErrorKind, JobEvent, JobId,
    RecoveryStatus, RevertResult, WriteXmlRequest, XmlNotListed, XmlWritten,
};
use sc_engine::export::{
    ExportJob, XmlOutRefusal, analysed_row, artefact_conflict, export_artefacts, failed_row,
    new_artefacts_folder, not_listed, run_export_job, xml_out_refusal, xml_tracks,
};
use sc_engine::{
    Analyzer, BatchFile, BatchRow, BatchSettings, CancelToken, EditState, EngineEvent, Place,
    ProcessSettings, Task, XmlSelect, apply_saved, check_inputs, run_batch, undo_file,
    write_artefacts,
};
use sc_io::rekordbox::is_soundcheck_xml;

use super::Shell;

/// `error` about the file `file_id`.
fn about(file_id: u32, error: Error) -> IpcError {
    IpcError {
        file_id: Some(file_id),
        ..IpcError::from(error)
    }
}

/// An absolute path from the UI.
fn absolute(path: &str, what: &str) -> Result<PathBuf, IpcError> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(Error::InvalidArgument(format!(
            "{what} must be an absolute path: {}",
            path.display()
        ))
        .into())
    }
}

impl Shell {
    /// Refuses while the start-up recovery runs.
    fn refuse_while_recovering(&self) -> Result<(), IpcError> {
        if self.inner.recovery.is_running() {
            return Err(IpcError {
                kind: IpcErrorKind::RecoveryRunning,
                message: "the check for changes a crash interrupted is still running; nothing was \
                          done"
                    .to_owned(),
                file_id: None,
            });
        }
        Ok(())
    }

    /// Where backups and the journal live.
    fn backup_root(&self) -> Result<PathBuf, IpcError> {
        self.inner.roots.backup.clone().ok_or_else(|| {
            Error::Internal("there is no backup folder (no home folder found)".into()).into()
        })
    }

    /// The rows `file_ids` name, in that order, each named once however it is spelled on disk.
    ///
    /// # Errors
    /// `invalidArgument` for no file or a row the session does not hold; `listedTwice` for a
    /// file named twice; `sameOutputName` for two files one folder would hold under one name.
    fn distinct_rows(&self, file_ids: &[u32], place: &Place) -> Result<Vec<BatchFile>, IpcError> {
        if file_ids.is_empty() {
            return Err(Error::InvalidArgument("no files were named".into()).into());
        }
        let files = {
            let session = self.session();
            if let Some(id) = file_ids.iter().find(|id| session.entry(**id).is_none()) {
                return Err(about(
                    *id,
                    Error::InvalidArgument(format!("row {id} is not in the list")),
                ));
            }
            session.batch(file_ids)
        };
        let paths: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
        let refused = check_inputs(&paths, place)
            .into_iter()
            .zip(&files)
            .find_map(|(refusal, file)| refusal.map(|e| about(file.file_id, e)));
        match refused {
            Some(error) => Err(error),
            None => Ok(files),
        }
    }

    /// Starts exporting `req.file_ids`, sending every event to `send`; returns at once with the
    /// job's id. A running job is cancelled with [`Shell::cancel`]. The grid view closes if it
    /// shows a file about to be replaced.
    ///
    /// # Errors
    /// Before anything is written: `recoveryRunning` while the start-up recovery runs;
    /// `invalidArgument` for settings out of their limits or a folder that does not fit the
    /// place; what [`Self::distinct_rows`] refuses; `alreadyExists` when a file SoundCheck did not
    /// write holds the name of an artefact in the folder.
    pub fn export(
        &self,
        req: ExportRequest,
        send: impl FnMut(JobEvent) + Send + 'static,
    ) -> Result<JobId, IpcError> {
        self.refuse_while_recovering()?;
        let out_dir = req
            .out_dir
            .as_deref()
            .map(|dir| absolute(dir, "the export folder"))
            .transpose()?;
        let process = ProcessSettings {
            decide: self.session().settings(),
            export: req.settings,
            out_dir: out_dir.clone(),
            backup_root: self.backup_root()?,
            edits: self.inner.edits.clone(),
            recovery: self.inner.recovery.clone(),
        };
        process.validate()?;
        let place = out_dir
            .as_ref()
            .map_or(Place::InPlace, |dir| Place::Folder(dir.clone()));
        self.distinct_rows(&req.file_ids, &place)?;
        if let Some(dir) = &out_dir {
            if dir.exists() && !dir.is_dir() {
                return Err(
                    Error::InvalidArgument(format!("{} is not a folder", dir.display())).into(),
                );
            }
            if req.settings.xml
                && let Some(conflict) = artefact_conflict(dir)
            {
                return Err(conflict.error.into());
            }
        }
        if req.settings.place == ExportPlace::InPlace {
            self.close_track_of(&req.file_ids);
        }
        let job = ExportJob {
            analysis: self.analysis_settings(),
            workers: self.inner.workers,
            cache: self.inner.cache.clone(),
            process,
            exports_root: self.inner.roots.exports.clone(),
        };
        let job_id = self.session().next_job_id();
        self.spawn_job(job_id, send, move |shell, cancel, forward| {
            run_export_job(
                &shell.inner.session,
                job_id,
                &req.file_ids,
                &job,
                cancel,
                forward,
            );
        });
        Ok(job_id)
    }

    /// Puts the original of the file's newest change back (`undo_file`, which verifies the backup
    /// and removes the file's cache entry) and refreshes its row: the original analysed again,
    /// which fills the cache, with its saved edit applied. The grid view closes if it shows the
    /// file. A failed analysis leaves the file reverted, with a note.
    ///
    /// # Errors
    /// `recoveryRunning`; `invalidArgument` for a row the session does not hold; `nothingToUndo`
    /// for a file with no change to revert; `fileChanged` for one edited since it was written;
    /// `verifyFailed`, the in-place refusals and `io` from the undo.
    pub fn revert(&self, file_id: u32) -> Result<RevertResult, IpcError> {
        self.revert_analysing(file_id, |path| self.analyse_original(path))
    }

    /// [`Self::revert`], with the original analysed by `analyse`.
    pub(super) fn revert_analysing(
        &self,
        file_id: u32,
        analyse: impl FnOnce(&Path) -> sc_core::Result<(AnalysisRecord, EditState)>,
    ) -> Result<RevertResult, IpcError> {
        self.refuse_while_recovering()?;
        let root = self.backup_root()?;
        let Some(entry) = self.session().entry(file_id) else {
            return Err(about(
                file_id,
                Error::InvalidArgument(format!("row {file_id} is not in the list")),
            ));
        };
        let path = PathBuf::from(&entry.path);
        let undone =
            undo_file(&path, &root, self.inner.cache.as_ref()).map_err(|e| about(file_id, e))?;
        self.close_track_of(&[file_id]);
        let mut notes = undone.notes;
        let analysis = match analyse(&path) {
            Ok(analysis) => Some(analysis),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "reverted file not analysed");
                notes.push(format!("the original could not be analysed again: {e}"));
                None
            }
        };
        let info = sc_io::probe(&path);
        let update = self
            .session()
            .set_reverted(file_id, info.clone(), analysis)
            .map(Box::new);
        Ok(RevertResult {
            entry: sc_core::ipc::FileEntry { info, ..entry },
            update,
            backup: undone.backup.display().to_string(),
            earlier_changes: u32::try_from(undone.earlier_changes).unwrap_or(u32::MAX),
            notes,
        })
    }

    /// The analysis of the file at `path`, with its saved edit applied.
    fn analyse_original(&self, path: &Path) -> sc_core::Result<(AnalysisRecord, EditState)> {
        let mut analyzer = Analyzer::load(
            self.analysis_settings(),
            self.inner.cache.clone(),
            CancelToken::new(),
        )?;
        let mut record = analyzer.analyze(path)?.record;
        let edit = self
            .inner
            .edits
            .as_ref()
            .map(|store| apply_saved(&mut record, store))
            .unwrap_or_default();
        Ok((record, edit))
    }

    /// Writes a rekordbox XML of the files `req.file_ids`, as `sc-cli xml` does: each analysed
    /// (the cache is used), with its saved edit, else its export's grid while the file is still
    /// what that export wrote, else its analysed grid; only grids the XML may carry are listed.
    /// Into a new folder under the exports folder, with the grid report, unless `req.out` names
    /// a file.
    ///
    /// # Errors
    /// `recoveryRunning`; what [`Self::distinct_rows`] refuses; `invalidArgument` for an `out`
    /// that is not an absolute `.xml` path; `alreadyExists` when a file SoundCheck did not write
    /// is there; `modelUnavailable`; `io` when writing fails. A file that cannot be analysed is
    /// reported among those not listed.
    pub fn write_xml(&self, req: WriteXmlRequest) -> Result<XmlWritten, IpcError> {
        self.refuse_while_recovering()?;
        let out = req
            .out
            .as_deref()
            .map(|out| absolute(out, "the rekordbox XML's file"))
            .transpose()?;
        if let Some(out) = &out {
            match xml_out_refusal(out) {
                Some(XmlOutRefusal::NotXml) => {
                    return Err(Error::InvalidArgument(format!(
                        "the rekordbox XML is written only to a file ending in .xml, not {}",
                        out.display()
                    ))
                    .into());
                }
                Some(XmlOutRefusal::Taken) => {
                    return Err(Error::AlreadyExists { path: out.clone() }.into());
                }
                None => {}
            }
        }
        let files = self.distinct_rows(&req.file_ids, &Place::InPlace)?;
        let rows = self.analysed_rows(&files)?;
        let artefacts = self.write_xml_rows(&rows, out.as_deref())?;
        let not_listed = not_listed(&rows, XmlSelect::All)
            .into_iter()
            .map(|(i, why)| XmlNotListed {
                file_id: files[i].file_id,
                why,
            })
            .collect();
        Ok(XmlWritten {
            artefacts,
            not_listed,
        })
    }

    /// The rows of `files`, analysed, in their order.
    fn analysed_rows(&self, files: &[BatchFile]) -> Result<Vec<BatchRow>, IpcError> {
        let settings = BatchSettings {
            analysis: self.analysis_settings(),
            workers: self.inner.workers,
            cache: self.inner.cache.clone(),
            task: Task::Analyze,
        };
        let decide = self.session().settings();
        let edits = self.inner.edits.clone();
        let mut rows: Vec<Option<BatchRow>> = files.iter().map(|_| None).collect();
        run_batch(files, &settings, &CancelToken::new(), &mut |event| {
            let (file_id, outcome) = match event {
                EngineEvent::Analysed {
                    file_id, report, ..
                } => (file_id, Ok(report)),
                EngineEvent::Failed { file_id, error } => (file_id, Err(error)),
                EngineEvent::Cancelled { file_id } => (file_id, Err(Error::Cancelled)),
                _ => return,
            };
            let Some(i) = files.iter().position(|f| f.file_id == file_id) else {
                return;
            };
            let file = &files[i].path;
            rows[i] = Some(match outcome {
                Ok(report) => analysed_row(file, *report, edits.as_ref(), &decide),
                Err(error) => failed_row(file, None, &error),
            });
        })?;
        Ok(rows.into_iter().flatten().collect())
    }

    /// Writes the XML of `rows` to `out`, or with the grid report into a new folder.
    fn write_xml_rows(
        &self,
        rows: &[BatchRow],
        out: Option<&Path>,
    ) -> Result<ExportArtefacts, IpcError> {
        let now = std::time::SystemTime::now();
        let playlist = sc_io::rekordbox::playlist_name(now);
        if let Some(out) = out {
            let tracks = xml_tracks(rows, XmlSelect::All);
            let bytes = sc_io::rekordbox::xml_bytes(&tracks, sc_core::VERSION, &playlist);
            sc_io::artefacts::write_artefact(out, &bytes, is_soundcheck_xml)?;
            return Ok(ExportArtefacts {
                folder: out
                    .parent()
                    .map_or_else(String::new, |dir| dir.display().to_string()),
                xml: Some(out.display().to_string()),
                report: None,
                listed: u32::try_from(tracks.len()).unwrap_or(u32::MAX),
            });
        }
        let dir = new_artefacts_folder(self.inner.roots.exports.as_deref(), now)?;
        let written = write_artefacts(&dir, rows, XmlSelect::All, &playlist)?;
        Ok(export_artefacts(&dir, &written))
    }

    /// Gives up on a change the start-up recovery left pending (see `sc_io::txn::forget`): the
    /// journal ends it and recovery skips it from then on. Nothing is deleted. The answer holds
    /// what stayed behind and the report without it.
    ///
    /// # Errors
    /// `recoveryRunning`; `invalidArgument` for a change recovery did not leave pending, or one
    /// that runs now; `io` when the journal cannot be read or written.
    pub fn forget(&self, txn: &str) -> Result<ForgetResult, IpcError> {
        self.refuse_while_recovering()?;
        let root = self.backup_root()?;
        let RecoveryStatus::Finished { recovered, pending } = self.recovery_status() else {
            return Err(not_pending(txn));
        };
        if !pending.iter().any(|p| p.txn == txn) {
            return Err(not_pending(txn));
        }
        let forgotten = sc_io::txn::forget(&root, txn)?;
        let status = RecoveryStatus::Finished {
            recovered,
            pending: pending.into_iter().filter(|p| p.txn != txn).collect(),
        };
        self.inner.recovery.set(status.clone());
        Ok(ForgetResult {
            path: forgotten.path.display().to_string(),
            backup: forgotten.backup.map(|b| b.display().to_string()),
            kept: forgotten
                .kept
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            status,
        })
    }
}

fn not_pending(txn: &str) -> IpcError {
    Error::InvalidArgument(format!("change {txn} is not left pending by recovery")).into()
}
