//! The command bodies, as plain Rust so they are tested without a window: the session, the jobs
//! in flight with their cancel tokens, and the cache. Each `#[tauri::command]` in `lib.rs` is a
//! one-line call into this.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use sc_core::Lufs;
use sc_core::analysis::AnalysisSettings;
use sc_core::ipc::{AnalyzeRequest, FileEntry, IpcError, JobEvent, JobId, Replan, SessionSnapshot};
use sc_core::plan::{DecideSettings, check_bpm_range};
use sc_engine::player::Player;
use sc_engine::{
    BatchSettings, CancelToken, Session, collect_audio_files, default_workers, probe_all, run_job,
};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;

/// State shared by every command.
#[derive(Clone)]
pub struct Shell {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) session: Mutex<Session>,
    jobs: Mutex<HashMap<JobId, CancelToken>>,
    pub(crate) cache: Option<Cache>,
    pub(crate) edits: Option<EditStore>,
    /// The analysis settings of the latest job, which the grid view's refits and cache lookups
    /// use.
    pub(crate) analysis: Mutex<AnalysisSettings>,
    /// The track open in the grid view.
    pub(crate) view: Mutex<Option<crate::grid_view::OpenTrack>>,
    /// The click player, started with the first track opened.
    pub(crate) player: Mutex<Option<Player>>,
    workers: usize,
}

impl Shell {
    /// A shell with an empty session deciding with the DJ defaults (the UI sends its persisted
    /// settings at start), analysing on `workers` threads with `cache` and applying the grid
    /// edits saved in `edits`.
    #[must_use]
    pub fn new(cache: Option<Cache>, edits: Option<EditStore>, workers: usize) -> Self {
        let session = Session::new(DecideSettings::dj());
        let session = match &edits {
            Some(store) => session.with_edits(store.clone()),
            None => session,
        };
        Self {
            inner: Arc::new(Inner {
                session: Mutex::new(session),
                jobs: Mutex::new(HashMap::new()),
                cache,
                edits,
                analysis: Mutex::new(AnalysisSettings::default()),
                view: Mutex::new(None),
                player: Mutex::new(None),
                workers,
            }),
        }
    }

    /// The shell the app runs: the user's analysis cache and saved grid edits, and the default
    /// worker count.
    #[must_use]
    pub fn for_app() -> Self {
        Self::new(
            Cache::default_dir().ok().map(Cache::open),
            EditStore::default_dir().ok().map(EditStore::open),
            default_workers(),
        )
    }

    pub(crate) fn session(&self) -> std::sync::MutexGuard<'_, Session> {
        self.inner
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, HashMap<JobId, CancelToken>> {
        self.inner
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds the audio files under `paths` (folders walked, headers probed) and returns the new
    /// rows; files already in the session are not added twice.
    #[must_use]
    pub fn expand(&self, paths: Vec<String>) -> Vec<FileEntry> {
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let files = collect_audio_files(&paths);
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let infos = probe_all(&files, threads);
        self.session().add(files.into_iter().zip(infos).collect())
    }

    /// Starts analysing `req.file_ids` on a background thread, sending every event to `send`;
    /// returns at once with the job's id.
    ///
    /// # Errors
    /// An `invalidArgument` error, and no job, when the BPM range is out of its limits.
    pub fn start(
        &self,
        req: AnalyzeRequest,
        mut send: impl FnMut(JobEvent) + Send + 'static,
    ) -> Result<JobId, IpcError> {
        check_bpm_range(req.analysis.bpm_range)?;
        *self
            .inner
            .analysis
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = req.analysis.clone();
        let job_id = self.session().next_job_id();
        let cancel = CancelToken::new();
        self.jobs().insert(job_id, cancel.clone());
        let shell = self.clone();
        let settings = BatchSettings {
            analysis: req.analysis,
            workers: self.inner.workers,
            cache: self.inner.cache.clone(),
        };
        std::thread::Builder::new()
            .name(format!("sc-job-{job_id}"))
            .spawn(move || {
                // The job leaves the running set before its last event goes out, so a caller
                // that has seen `finished` never finds it still cancellable.
                let jobs = shell.clone();
                let mut forward = |event: JobEvent| {
                    if matches!(event, JobEvent::Finished { .. }) {
                        jobs.jobs().remove(&job_id);
                    }
                    send(event);
                };
                run_job(
                    &shell.inner.session,
                    job_id,
                    &req.file_ids,
                    &settings,
                    &cancel,
                    &mut forward,
                );
                shell.jobs().remove(&job_id);
            })
            .expect("the OS can start a thread for a job");
        Ok(job_id)
    }

    /// Asks a running job to stop; `false` when no such job is running.
    #[must_use]
    pub fn cancel(&self, job_id: JobId) -> bool {
        self.jobs().get(&job_id).map(CancelToken::cancel).is_some()
    }

    /// Changes the decide settings; returns every analysed row's new plan and its revision.
    ///
    /// # Errors
    /// An `invalidArgument` error when a value is out of its limits; nothing changes then.
    pub fn set_decide_settings(&self, settings: DecideSettings) -> Result<Replan, IpcError> {
        Ok(self.session().set_settings(settings)?)
    }

    /// Empties the track list: running jobs are cancelled and every file is forgotten, so the
    /// same files can be added again. The disk cache keeps their analyses.
    pub fn clear(&self) {
        for token in self.jobs().values() {
            token.cancel();
        }
        self.session().clear();
    }

    /// Everything the session holds, for a window that reloads. Jobs still running are
    /// cancelled: their events would go to a page that no longer listens.
    #[must_use]
    pub fn restore(&self) -> SessionSnapshot {
        for token in self.jobs().values() {
            token.cancel();
        }
        self.session().snapshot()
    }

    /// The median S-P95 of the analysed rows, for "Calibrate from my library".
    #[must_use]
    pub fn calibration_target(&self) -> Option<Lufs> {
        self.session().median_short_term_p95()
    }
}

#[cfg(test)]
mod tests;
