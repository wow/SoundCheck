//! The export commands' bodies: saying what an export would do, starting one, reverting a file,
//! writing a rekordbox XML of analysed rows and forgetting a change recovery left pending. Every
//! one of them is refused while the start-up recovery runs: the engine would only wait for it, and
//! the user is better told.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sc_core::export::Place as ExportPlace;
use sc_core::ipc::{
    ExportArtefacts, ExportPreview, ExportPreviewRow, ExportRequest, ExportedWith, ForgetResult,
    IpcError, IpcErrorKind, JobEvent, JobId, RecoveryStatus, RevertResult, WriteXmlRequest,
    XmlNotListed, XmlWritten,
};
use sc_core::{ChangeCause, Error};
use sc_engine::export::{
    ExportJob, PreviewInput, XmlOutRefusal, analysed_row, artefact_conflict, export_artefacts,
    failed_row, new_artefacts_folder, not_listed, preview_row, run_export_job, tracks_listed_in,
    xml_out_refusal, xml_tracks,
};
use sc_engine::{
    Analyzer, BatchFile, BatchRow, BatchSettings, CancelToken, EngineEvent, FolderNames, Place,
    ProcessSettings, RevertedAnalysis, Task, XmlSelect, apply_saved, check_inputs, run_batch,
    undo_file, write_artefacts,
};
use sc_io::rekordbox::is_soundcheck_xml;

use super::{HoldKind, Shell};

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

    /// The rows `file_ids` name, in that order, each named once however it is spelled on disk, and
    /// the later of any files one folder would hold under one name, going by their paths alone (the
    /// plans, which know a row that writes nothing takes no name, come after this).
    ///
    /// # Errors
    /// `invalidArgument` for no file or a row the session does not hold; `listedTwice` for a
    /// file named twice.
    fn distinct_rows(
        &self,
        file_ids: &[u32],
        place: &Place,
    ) -> Result<(Vec<BatchFile>, BTreeSet<u32>), IpcError> {
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
        let mut same_name = BTreeSet::new();
        for (refusal, file) in check_inputs(&paths, place).into_iter().zip(&files) {
            match refusal {
                Some(Error::SameOutputName { .. }) => {
                    same_name.insert(file.file_id);
                }
                Some(error) => return Err(about(file.file_id, error)),
                None => {}
            }
        }
        Ok((files, same_name))
    }

    /// Refuses a folder nothing is written into, whatever the rows do: inside the backups, which
    /// SoundCheck never lists or touches again, and on a rekordbox USB export.
    ///
    /// # Errors
    /// `invalidArgument`, `rekordboxUsbExport`, or `io` when no folder above it resolves.
    fn refuse_destination(&self, dir: &Path) -> Result<(), IpcError> {
        if sc_io::txn::is_under_backup_root(dir, &self.backup_root()?)? {
            return Err(Error::InvalidArgument(format!(
                "{} is inside the backup folder; choose another folder",
                dir.display()
            ))
            .into());
        }
        sc_io::txn::check_folder_not_rekordbox(dir, &sc_io::txn::SystemVolumes)?;
        Ok(())
    }

    /// What an export of `req` needs, once everything that refuses it before anything is written
    /// has had its say.
    ///
    /// # Errors
    /// See [`Self::export`], except `busy`.
    fn checked_export(
        &self,
        req: &ExportRequest,
    ) -> Result<(ProcessSettings, Vec<BatchFile>, u32), IpcError> {
        self.refuse_while_recovering()?;
        let out_dir = req
            .out_dir
            .as_deref()
            .map(|dir| absolute(dir, "the export folder"))
            .transpose()?;
        // One read, so the numbers and the revision they are checked by go together.
        let (decide, revision) = {
            let session = self.session();
            (session.settings(), session.revision())
        };
        let process = ProcessSettings {
            decide,
            export: req.settings,
            out_dir: out_dir.clone(),
            backup_root: self.backup_root()?,
            edits: self.inner.edits.clone(),
            recovery: self.inner.recovery.clone(),
            confirmed: req.confirmed.as_ref().map(|c| {
                c.files
                    .iter()
                    .map(|f| (f.file_id, f.identity.clone()))
                    .collect()
            }),
            same_name: BTreeSet::new(),
        };
        process.validate()?;
        let place = out_dir
            .as_ref()
            .map_or(Place::InPlace, |dir| Place::Folder(dir.clone()));
        let (files, by_path) = self.distinct_rows(&req.file_ids, &place)?;
        let process = ProcessSettings {
            // The confirm step decided who takes a name, knowing which rows write; what the user
            // was told it would skip stays skipped, whatever dropped out of the job since.
            same_name: req.confirmed.as_ref().map_or(by_path, |c| {
                let skipped = c.files.iter().filter(|f| f.same_name);
                skipped.map(|f| f.file_id).collect()
            }),
            ..process
        };
        if let Some(dir) = &out_dir {
            if dir.exists() && !dir.is_dir() {
                return Err(
                    Error::InvalidArgument(format!("{} is not a folder", dir.display())).into(),
                );
            }
            self.refuse_destination(dir)?;
            if req.settings.xml
                && let Some(conflict) = artefact_conflict(dir)
            {
                return Err(conflict.error.into());
            }
        }
        Ok((process, files, revision))
    }

    /// What exporting `req.file_ids` would do with each, planned as the job plans them, with
    /// everything the job would refuse refused here too. Nothing is written. A file that cannot be
    /// planned (not analysed yet, changed since it was analysed, its start unreadable, an export in
    /// place refusing it) gets its error on its row, as the job would report it on its row. Of the
    /// rows that plan a write, the first in the list takes a name and the later ones are skipped.
    ///
    /// # Errors
    /// What [`Self::export`] refuses; `busy` while one of the files is being exported or reverted.
    pub fn plan_export(&self, req: &ExportRequest) -> Result<ExportPreview, IpcError> {
        let (process, files, revision) = self.checked_export(req)?;
        let _hold = self.hold(&files, HoldKind::Reading)?;
        let mut names = process.out_dir.as_deref().map(FolderNames::new);
        let rows = files
            .iter()
            .map(|file| self.preview_of(file, &process, req, names.as_mut()))
            .collect();
        let shown = |dir: &Path| dir.display().to_string();
        let artefacts_folder = if req.settings.xml {
            match &process.out_dir {
                Some(dir) => Some(shown(dir)),
                None => self.inner.roots.exports.as_deref().map(shown),
            }
        } else {
            None
        };
        let xml_listed = process
            .out_dir
            .as_deref()
            .filter(|_| req.settings.xml)
            .map_or(0, tracks_listed_in);
        Ok(ExportPreview {
            rows,
            revision,
            decide: process.decide,
            xml_listed: u32::try_from(xml_listed).unwrap_or(u32::MAX),
            backup_folder: process
                .out_dir
                .is_none()
                .then(|| shown(&process.backup_root)),
            artefacts_folder,
        })
    }

    /// What exporting `file` would do, or why that cannot be said.
    fn preview_of(
        &self,
        file: &BatchFile,
        process: &ProcessSettings,
        req: &ExportRequest,
        names: Option<&mut FolderNames>,
    ) -> ExportPreviewRow {
        let file_id = file.file_id;
        let failed = |error: Error| ExportPreviewRow {
            file_id,
            row: None,
            error: Some(about(file_id, error)),
            identity: None,
            replaces_copy: false,
        };
        let Some(planning) = self.session().planning_row(file_id) else {
            return failed(Error::InvalidArgument(format!(
                "{} is not analysed yet",
                file.path.display()
            )));
        };
        // Read first: a file that changes during the planning is told apart by the job.
        let identity = match sc_io::txn::file_identity(&file.path) {
            Ok(identity) => identity,
            Err(error) => return failed(error),
        };
        if planning.identity.as_ref().is_some_and(|i| *i != identity) {
            return failed(Error::FileChanged {
                path: file.path.clone(),
                detail: "since it was analysed; press Analyse to read it again".to_owned(),
                cause: ChangeCause::SincePlanned,
            });
        }
        let input = PreviewInput {
            path: &file.path,
            record: &planning.record,
            edit: planning.edit,
            decide: &process.decide,
            settings: &req.settings,
            out_dir: process.out_dir.as_deref(),
            revertible: planning.revertible,
            backup_root: &process.backup_root,
        };
        match preview_row(&input, names) {
            Ok((mut row, replaces_copy)) => {
                row.exported_with = Some(ExportedWith::new(
                    process.decide,
                    req.settings,
                    process.out_dir.as_deref(),
                ));
                ExportPreviewRow {
                    file_id,
                    row: Some(row),
                    error: None,
                    identity: Some(identity),
                    replaces_copy,
                }
            }
            Err(error) => failed(error),
        }
    }

    /// Starts exporting `req.file_ids`, sending every event to `send`; returns at once with the
    /// job's id. A running job is cancelled with [`Shell::cancel`]. The grid view closes if it
    /// shows a file about to be replaced.
    ///
    /// # Errors
    /// Before anything is written: `recoveryRunning` while the start-up recovery runs;
    /// `invalidArgument` for settings out of their limits or a folder that does not fit the
    /// place; what [`Self::distinct_rows`] and [`Self::refuse_destination`] refuse; `alreadyExists`
    /// when a file SoundCheck did not write holds the name of an artefact in the folder; `busy`
    /// when another job or command is working on one of the files.
    pub fn export(
        &self,
        req: ExportRequest,
        send: impl FnMut(JobEvent) + Send + 'static,
    ) -> Result<JobId, IpcError> {
        let (process, files, revision) = self.checked_export(&req)?;
        if req
            .confirmed
            .as_ref()
            .is_some_and(|c| c.revision != revision)
        {
            return Err(IpcError {
                kind: IpcErrorKind::SettingsChanged,
                message: "the loudness settings changed after this plan; nothing was done"
                    .to_owned(),
                file_id: None,
            });
        }
        let kind = if req.settings.place == ExportPlace::InPlace {
            HoldKind::ExportInPlace
        } else {
            HoldKind::ExportCopy
        };
        let hold = self.hold(&files, kind)?;
        if kind == HoldKind::ExportInPlace {
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
        self.spawn_job(job_id, hold, send, move |shell, cancel, forward| {
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
    /// which fills the cache, with its saved edit applied. The file is held throughout, so a
    /// second press cannot undo a second change. The grid view closes if it shows the file. A
    /// failed analysis leaves the file reverted, with a note.
    ///
    /// # Errors
    /// `recoveryRunning`; `invalidArgument` for a row the session does not hold; `nothingToUndo`
    /// for a file with no change to revert; `fileChanged` for one edited since it was written;
    /// `busy` while another job or command is working on it; `verifyFailed`, the in-place refusals
    /// and `io` from the undo.
    pub fn revert(&self, file_id: u32) -> Result<RevertResult, IpcError> {
        self.revert_analysing(file_id, |path| self.analyse_original(path))
    }

    /// [`Self::revert`], with the original analysed by `analyse`.
    pub(super) fn revert_analysing(
        &self,
        file_id: u32,
        analyse: impl FnOnce(&Path) -> sc_core::Result<RevertedAnalysis>,
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
        let _hold = self
            .inner
            .holds
            .acquire(&[(file_id, path.as_path())], HoldKind::Revert)?;
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
            .set_reverted(file_id, info.clone(), analysis, undone.earlier_changes)
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
    fn analyse_original(&self, path: &Path) -> sc_core::Result<RevertedAnalysis> {
        let mut analyzer = Analyzer::load(
            self.analysis_settings(),
            self.inner.cache.clone(),
            CancelToken::new(),
        )?;
        let report = analyzer.analyze(path)?;
        let mut record = report.record;
        let edit = self
            .inner
            .edits
            .as_ref()
            .map(|store| apply_saved(&mut record, store))
            .unwrap_or_default();
        Ok(RevertedAnalysis {
            record,
            edit,
            identity: report.identity,
            bpm_range: report.bpm_range,
        })
    }

    /// Writes a rekordbox XML of the files `req.file_ids`, as `sc-cli xml` does: each analysed
    /// (the cache is used), with its saved edit, else its export's grid while the file is still
    /// what that export wrote, else its analysed grid; only grids the XML may carry are listed.
    /// Into a new folder under the exports folder, with the grid report, unless `req.out` names
    /// a file.
    ///
    /// # Errors
    /// `recoveryRunning`; what [`Self::distinct_rows`] refuses; `invalidArgument` for an `out`
    /// that is not an absolute `.xml` path; what [`Self::refuse_destination`] refuses;
    /// `alreadyExists` when a file SoundCheck did not write is there; `busy` while one of the
    /// files is being exported or reverted; `modelUnavailable`; `io` when writing fails. A file
    /// that cannot be analysed is reported among those not listed.
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
            if let Some(dir) = out.parent() {
                self.refuse_destination(dir)?;
            }
        }
        let (files, _) = self.distinct_rows(&req.file_ids, &Place::InPlace)?;
        let _hold = self.hold(&files, HoldKind::Reading)?;
        // A file exported to a folder is listed as that copy, which is what the export made.
        let files: Vec<BatchFile> = {
            let session = self.session();
            files
                .into_iter()
                .map(|file| match session.exported_copy(file.file_id) {
                    Some(copy) if copy.is_file() => BatchFile { path: copy, ..file },
                    _ => file,
                })
                .collect()
        };
        let rows = self.analysed_rows(&files)?;
        let artefacts = self.write_xml_rows(&rows, out.as_deref())?;
        self.session().set_artefacts(Some(Ok(artefacts.clone())));
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
        let listed = matches!(
            self.recovery_status(),
            RecoveryStatus::Finished { pending, .. } if pending.iter().any(|p| p.txn == txn)
        );
        if !listed {
            return Err(not_pending(txn));
        }
        let forgotten = sc_io::txn::forget(&root, txn)?;
        let status = self.inner.recovery.update(|status| {
            if let RecoveryStatus::Finished { pending, .. } = status {
                pending.retain(|p| p.txn != txn);
            }
        });
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
