//! The command bodies, as plain Rust so they are tested without a window: the session, the jobs
//! in flight with their cancel tokens, and the cache. Each `#[tauri::command]` in `lib.rs` is a
//! one-line call into this.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use sc_core::Lufs;
use sc_core::ipc::{AnalyzeRequest, FileEntry, IpcError, JobEvent, JobId, Replan, SessionSnapshot};
use sc_core::plan::{DecideSettings, check_bpm_range};
use sc_engine::{
    BatchSettings, CancelToken, Session, collect_audio_files, default_workers, probe_all, run_job,
};
use sc_io::cache::Cache;

/// State shared by every command.
#[derive(Clone)]
pub struct Shell {
    inner: Arc<Inner>,
}

struct Inner {
    session: Mutex<Session>,
    jobs: Mutex<HashMap<JobId, CancelToken>>,
    cache: Option<Cache>,
    workers: usize,
}

impl Shell {
    /// A shell with an empty session deciding with the DJ defaults (the UI sends its persisted
    /// settings at start), analysing on `workers` threads with `cache`.
    #[must_use]
    pub fn new(cache: Option<Cache>, workers: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                session: Mutex::new(Session::new(DecideSettings::dj())),
                jobs: Mutex::new(HashMap::new()),
                cache,
                workers,
            }),
        }
    }

    /// The shell the app runs: the user's analysis cache and the default worker count.
    #[must_use]
    pub fn for_app() -> Self {
        Self::new(
            Cache::default_dir().ok().map(Cache::open),
            default_workers(),
        )
    }

    fn session(&self) -> std::sync::MutexGuard<'_, Session> {
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
mod tests {
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    use sc_core::analysis::{AnalysisSettings, Model};
    use sc_core::plan::GainPlan;
    use sc_core::{AudioSpec, Bpm, testsig};

    use super::*;

    fn tone_wav(dir: &Path, name: &str, amp: f32) {
        let buf = testsig::sine(AudioSpec::CD, 1000.0, amp, 5.0);
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(dir.join(name), spec).unwrap();
        for s in &buf.data {
            #[allow(clippy::cast_possible_truncation)]
            w.write_sample((f64::from(*s) * f64::from(i16::MAX)).round() as i16)
                .unwrap();
        }
        w.finalize().unwrap();
    }

    fn loudness_only() -> AnalysisSettings {
        AnalysisSettings {
            bpm_range: (Bpm(70.0), Bpm(180.0)),
            grid: false,
            model: Model::Small,
        }
    }

    /// Collects a job's events until `finished`.
    fn events_until_finished(rx: &mpsc::Receiver<JobEvent>) -> Vec<JobEvent> {
        let mut events = Vec::new();
        loop {
            let e = rx
                .recv_timeout(Duration::from_secs(30))
                .expect("the job ends");
            let done = matches!(e, JobEvent::Finished { .. });
            events.push(e);
            if done {
                return events;
            }
        }
    }

    #[test]
    fn a_dropped_folder_becomes_rows_then_analysed_rows_with_plans() {
        let dir = tempfile::tempdir().unwrap();
        tone_wav(dir.path(), "a.wav", 0.1);
        tone_wav(dir.path(), "b.wav", 0.3);
        let shell = Shell::new(None, 2);
        let rows = shell.expand(vec![dir.path().display().to_string()]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].info.sample_rate, Some(44_100));
        assert!(
            shell
                .expand(vec![dir.path().display().to_string()])
                .is_empty(),
            "no duplicates"
        );

        let (tx, rx) = mpsc::channel();
        let req = AnalyzeRequest {
            file_ids: rows.iter().map(|r| r.file_id).collect(),
            analysis: loudness_only(),
        };
        let job = shell.start(req, move |e| tx.send(e).unwrap()).unwrap();
        let events = events_until_finished(&rx);
        assert!(
            matches!(events.last(), Some(JobEvent::Finished { job_id, cancelled: false }) if *job_id == job)
        );
        let analysed = events
            .iter()
            .filter(|e| matches!(e, JobEvent::Analysed { .. }))
            .count();
        assert_eq!(analysed, 2);
        assert!(!shell.cancel(job), "a finished job cannot be cancelled");

        let replan = shell
            .set_decide_settings(DecideSettings::streaming())
            .unwrap();
        assert_eq!(replan.plans.len(), 2);
        assert!(
            replan
                .plans
                .iter()
                .all(|p| matches!(p.plan.gain, Some(GainPlan::Gain { .. })))
        );
        assert!(shell.calibration_target().is_some());
        let snapshot = shell.restore();
        assert_eq!(snapshot.rows.len(), 2);
        assert!(snapshot.rows.iter().all(|r| r.plan.is_some()));
    }

    #[test]
    fn cancel_stops_a_running_job() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            tone_wav(dir.path(), &format!("{i}.wav"), 0.1);
        }
        let shell = Shell::new(None, 1);
        let rows = shell.expand(vec![dir.path().display().to_string()]);
        let (tx, rx) = mpsc::channel();
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let req = AnalyzeRequest {
            file_ids: rows.iter().map(|r| r.file_id).collect(),
            analysis: loudness_only(),
        };
        // Hold the job at its first event until Cancel has been pressed, so the cancel lands
        // while at most the first file is in flight.
        let mut gate = Some(gate_rx);
        let job = shell
            .start(req, move |e| {
                let _ = tx.send(e);
                if let Some(gate) = gate.take() {
                    let _ = gate.recv();
                }
            })
            .unwrap();
        let started = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(matches!(started, JobEvent::Started { .. }), "{started:?}");
        assert!(shell.cancel(job), "the job is running");
        gate_tx.send(()).unwrap();
        let events = events_until_finished(&rx);
        assert!(matches!(
            events.last(),
            Some(JobEvent::Finished {
                cancelled: true,
                ..
            })
        ));
        let count = |f: fn(&JobEvent) -> bool| events.iter().filter(|e| f(e)).count();
        let cancelled = count(|e| matches!(e, JobEvent::Cancelled { .. }));
        let analysed = count(|e| matches!(e, JobEvent::Analysed { .. }));
        assert_eq!(cancelled + analysed, 6, "every row ends");
        assert!(cancelled >= 4, "{cancelled} cancelled");
    }

    #[test]
    fn out_of_range_settings_are_refused() {
        let shell = Shell::new(None, 1);
        let bad = DecideSettings {
            ceiling: sc_core::DbTp(0.5),
            ..DecideSettings::dj()
        };
        let err = shell.set_decide_settings(bad).unwrap_err();
        assert_eq!(err.kind, sc_core::ipc::IpcErrorKind::InvalidArgument);
        let mut analysis = loudness_only();
        analysis.bpm_range = (Bpm(180.0), Bpm(70.0));
        let req = AnalyzeRequest {
            file_ids: vec![],
            analysis,
        };
        assert!(shell.start(req, |_| {}).is_err());
    }

    #[test]
    fn a_cleared_list_takes_the_same_files_again() {
        let dir = tempfile::tempdir().unwrap();
        tone_wav(dir.path(), "a.wav", 0.1);
        let shell = Shell::new(None, 1);
        let first = shell.expand(vec![dir.path().display().to_string()]);
        assert_eq!(first.len(), 1);
        shell.clear();
        assert!(shell.restore().rows.is_empty());
        let again = shell.expand(vec![dir.path().display().to_string()]);
        assert_eq!(again.len(), 1);
        assert_ne!(again[0].file_id, first[0].file_id, "ids are not reused");
    }

    #[test]
    fn cancelling_an_unknown_job_says_so() {
        assert!(!Shell::new(None, 1).cancel(42));
    }
}
