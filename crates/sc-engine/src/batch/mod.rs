//! Many files at once: a fixed set of worker threads, each with its own analyzer and beat model,
//! takes files in order from a shared counter. Workers send their events over a channel to the
//! calling thread, which forwards them to `on_event`, so the callback never runs concurrently
//! and needs no locking.
//!
//! A batch either analyses its files ([`Task::Analyze`]) or exports them ([`Task::Process`]:
//! analysis, plan, write, the output's own analysis; see [`crate::export::process`]). An export
//! waits for the start-up recovery before it touches any file.
//!
//! Every file gets exactly one terminal event (analysed, written and done, skipped by the
//! export, failed or cancelled), preceded by `Started` and progress at most every 100 ms; an
//! export sends `Processing` and `Written` before `Done`. A batch summary follows at most every
//! 500 ms and once more at the end. A failing file never stops the batch; a missing model or
//! invalid export settings stop it before any file. Records do not depend on the worker count.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use sc_core::analysis::AnalysisSettings;
use sc_core::export::{ExportOutcome, ExportPlan};
use sc_core::{Error, Result, Seconds};
use sc_io::cache::Cache;
use sc_io::txn::TxnReport;

use crate::analyze::{AnalyzeReport, Analyzer, Progress, Timings};
use crate::cancel::CancelToken;
use crate::edits::EditState;
use crate::export::process::process_file;
use crate::export::{ProcessDone, ProcessSettings};

/// Shortest gap between two progress events of one file.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Shortest gap between two batch summaries.
const SUMMARY_INTERVAL: Duration = Duration::from_millis(500);

/// Upper bound on the default worker count: each worker holds a beat model and peaks near
/// 450 MB while tracking.
const MAX_DEFAULT_WORKERS: usize = 4;

/// One file of a batch.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchFile {
    /// The caller's id, echoed in every event about this file.
    pub file_id: u32,
    /// The file.
    pub path: PathBuf,
    /// Playing time in seconds if already known (from a tag probe), for the batch ETA.
    pub duration_hint: Option<f64>,
}

/// How a batch runs.
#[derive(Debug, Clone)]
pub struct BatchSettings {
    /// Analysis settings, shared by every file.
    pub analysis: AnalysisSettings,
    /// Worker threads; clamped to `1..=files`.
    pub workers: usize,
    /// The cache to consult and fill; `None` bypasses it.
    pub cache: Option<Cache>,
    /// What the batch does with each file.
    pub task: Task,
}

/// What a batch does with each file.
#[derive(Debug, Clone, Default)]
pub enum Task {
    /// Analyse it ([`EngineEvent::Analysed`]).
    #[default]
    Analyze,
    /// Export it ([`EngineEvent::Done`] or [`EngineEvent::ExportSkipped`]).
    Process(Box<ProcessSettings>),
}

/// What a batch reports, in the order it happens.
#[derive(Debug)]
pub enum EngineEvent {
    /// A worker took the file.
    Started {
        /// The file.
        file_id: u32,
    },
    /// Analysis progress of one file, `0.0..=1.0`, at most every 100 ms.
    Progress {
        /// The file.
        file_id: u32,
        /// Fraction done.
        fraction: f32,
    },
    /// Terminal: the analysis, fresh or from the cache.
    Analysed {
        /// The file.
        file_id: u32,
        /// The report.
        report: Box<AnalyzeReport>,
        /// What a saved grid edit did to the report's grid (none from the batch itself; the
        /// app's job applies the user's edits before the session stores the row).
        edit: EditState,
    },
    /// Terminal: the file could not be analysed; the batch goes on.
    Failed {
        /// The file.
        file_id: u32,
        /// Why.
        error: Error,
    },
    /// Terminal: the batch was cancelled before or while this file ran; an export of it never
    /// replaced anything.
    Cancelled {
        /// The file.
        file_id: u32,
    },
    /// Export: the file is analysed and planned, and its write starts.
    Processing {
        /// The file.
        file_id: u32,
        /// What is written, planned from the cut the renderer makes.
        plan: Box<ExportPlan>,
    },
    /// Export: the output is written and verified (in place, the original backed up first).
    Written {
        /// The file.
        file_id: u32,
        /// What the transaction did.
        report: Box<TxnReport>,
    },
    /// Terminal (export): the file is not written: skipped, or left to the rekordbox XML.
    ExportSkipped {
        /// The file.
        file_id: u32,
        /// Why ([`ExportOutcome::Skip`] or [`ExportOutcome::XmlOnly`]).
        outcome: Box<ExportOutcome>,
        /// Its playing time.
        duration: Seconds,
    },
    /// Terminal (export): written, then the grid edit carried over and the output analysed.
    Done {
        /// The file.
        file_id: u32,
        /// What became of the written file.
        done: Box<ProcessDone>,
    },
    /// Where the whole batch stands.
    Batch(BatchProgress),
}

/// Where a batch stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchProgress {
    /// Files with a terminal event.
    pub done: usize,
    /// Files in the batch.
    pub total: usize,
    /// Remaining wall time, once a file has finished.
    pub eta: Option<Duration>,
    /// Seconds of audio analysed per second of wall time so far, once a file has finished.
    pub realtime_x: Option<f64>,
}

/// How a batch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchSummary {
    /// Files analysed (fresh or cached).
    pub analysed: usize,
    /// Files an export wrote.
    pub written: usize,
    /// Files an export did not write (skipped, or left to the rekordbox XML).
    pub not_written: usize,
    /// Files that failed.
    pub failed: usize,
    /// Files cancelled.
    pub cancelled: usize,
    /// Whether the batch was cancelled.
    pub was_cancelled: bool,
}

/// The default worker count: a quarter of the logical cores, at most [`MAX_DEFAULT_WORKERS`].
///
/// The beat model already spreads each file's inference over every core, so more files at once
/// mostly add contention: on an 8-core M1, 20 tracks took 120 s with one worker, 88 s with two,
/// 90 s with three and 94 s with four, while memory grew by about 440 MB per worker.
#[must_use]
pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get() / 4)
        .clamp(1, MAX_DEFAULT_WORKERS)
}

/// A worker's message: the file's index in the batch and what happened.
type Message = (usize, EngineEvent);

/// Runs `settings.task` on `files` with `settings.workers` threads, calling `on_event` from the
/// calling thread. An export first waits for the start-up recovery
/// ([`ProcessSettings::recovery`]); a cancel while it waits cancels every file.
///
/// # Errors
/// Only errors that stop the batch before any file: [`Error::ModelUnavailable`] or
/// [`Error::Internal`] while loading the beat model, [`Error::InvalidArgument`] for export
/// settings out of their limits. Per-file errors arrive as [`EngineEvent::Failed`].
pub fn run_batch(
    files: &[BatchFile],
    settings: &BatchSettings,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(EngineEvent),
) -> Result<BatchSummary> {
    if let Task::Process(process) = &settings.task {
        process.validate()?;
    }
    let workers = settings.workers.clamp(1, files.len().max(1));
    let analyzers = (0..workers)
        .map(|_| {
            Analyzer::load(
                settings.analysis.clone(),
                settings.cache.clone(),
                cancel.clone(),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let mut state = Coordinator::new(files);
    // No file is touched while recovery may still repair it; a cancel meanwhile leaves every
    // file to the loop below.
    let gate_open = match &settings.task {
        Task::Process(process) => process.recovery.wait(cancel).is_ok(),
        Task::Analyze => true,
    };

    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<Message>();
        for analyzer in analyzers.into_iter().filter(|_| gate_open) {
            let tx = tx.clone();
            let next = &next;
            let task = &settings.task;
            scope.spawn(move || work(analyzer, task, files, next, &tx));
        }
        drop(tx);
        let mut last_summary = Instant::now();
        loop {
            match rx.recv_timeout(SUMMARY_INTERVAL) {
                Ok((index, event)) => state.forward(index, event, on_event),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if last_summary.elapsed() >= SUMMARY_INTERVAL {
                on_event(EngineEvent::Batch(state.progress(started.elapsed())));
                last_summary = Instant::now();
            }
        }
    });

    // Files no worker reached: cancelled if the batch was, else (never expected) failed.
    for (index, file) in files.iter().enumerate() {
        if !state.terminal[index] {
            let event = if cancel.is_cancelled() {
                EngineEvent::Cancelled {
                    file_id: file.file_id,
                }
            } else {
                EngineEvent::Failed {
                    file_id: file.file_id,
                    error: Error::Internal("no worker reached the file".into()),
                }
            };
            state.forward(index, event, on_event);
        }
    }
    on_event(EngineEvent::Batch(state.progress(started.elapsed())));
    state.summary.was_cancelled = cancel.is_cancelled();
    Ok(state.summary)
}

/// One worker: takes the next file until none are left or the batch is cancelled.
fn work(
    mut analyzer: Analyzer,
    task: &Task,
    files: &[BatchFile],
    next: &AtomicUsize,
    tx: &mpsc::Sender<Message>,
) {
    loop {
        if analyzer.cancel.is_cancelled() {
            return;
        }
        let index = next.fetch_add(1, Ordering::Relaxed);
        let Some(file) = files.get(index) else { return };
        let file_id = file.file_id;
        // A send fails only when the batch has returned; nothing is left to report to.
        let _ = tx.send((index, EngineEvent::Started { file_id }));
        let progress = throttled_progress(index, file_id, tx.clone());
        let outcome = catch_unwind(AssertUnwindSafe(|| match task {
            Task::Analyze => analyzer
                .analyze_with(&file.path, &mut Timings::default(), Some(&progress))
                .map(|report| EngineEvent::Analysed {
                    file_id,
                    report: Box::new(report),
                    edit: EditState::default(),
                }),
            Task::Process(settings) => {
                let emit = |event| {
                    let _ = tx.send((index, event));
                };
                process_file(
                    &mut analyzer,
                    file_id,
                    &file.path,
                    settings,
                    Some(&progress),
                    &emit,
                )
            }
        }));
        let event = match outcome {
            Ok(Ok(event)) => event,
            Ok(Err(Error::Cancelled)) => EngineEvent::Cancelled { file_id },
            Ok(Err(error)) => EngineEvent::Failed { file_id, error },
            Err(_) => EngineEvent::Failed {
                file_id,
                error: Error::Internal(match task {
                    Task::Analyze => "the analysis of this file crashed".into(),
                    Task::Process(_) => "the export of this file crashed".into(),
                }),
            },
        };
        let _ = tx.send((index, event));
    }
}

/// A progress sink that sends at most one event per [`PROGRESS_INTERVAL`].
fn throttled_progress(index: usize, file_id: u32, tx: mpsc::Sender<Message>) -> Progress {
    let last: Mutex<Option<Instant>> = Mutex::new(None);
    Arc::new(move |fraction| {
        let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|t| t.elapsed() < PROGRESS_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
        let _ = tx.send((index, EngineEvent::Progress { file_id, fraction }));
    })
}

/// The calling thread's view of the batch.
struct Coordinator {
    terminal: Vec<bool>,
    hints: Vec<Option<f64>>,
    audio_done: f64,
    summary: BatchSummary,
}

impl Coordinator {
    fn new(files: &[BatchFile]) -> Self {
        Self {
            terminal: vec![false; files.len()],
            hints: files.iter().map(|f| f.duration_hint).collect(),
            audio_done: 0.0,
            summary: BatchSummary::default(),
        }
    }

    fn forward(&mut self, index: usize, event: EngineEvent, on_event: &mut dyn FnMut(EngineEvent)) {
        let terminal = match &event {
            EngineEvent::Analysed { report, .. } => {
                self.summary.analysed += 1;
                self.audio_done += report.record.duration.0;
                true
            }
            EngineEvent::Failed { .. } => {
                self.summary.failed += 1;
                true
            }
            EngineEvent::Cancelled { .. } => {
                self.summary.cancelled += 1;
                true
            }
            EngineEvent::Done { done, .. } => {
                self.summary.written += 1;
                self.audio_done += done.duration.0;
                true
            }
            EngineEvent::ExportSkipped { duration, .. } => {
                self.summary.not_written += 1;
                self.audio_done += duration.0;
                true
            }
            EngineEvent::Started { .. }
            | EngineEvent::Progress { .. }
            | EngineEvent::Processing { .. }
            | EngineEvent::Written { .. }
            | EngineEvent::Batch(_) => false,
        };
        if terminal {
            self.terminal[index] = true;
        }
        on_event(event);
    }

    /// Files that ended with their audio measured: analysed, written or not written.
    fn finished(&self) -> usize {
        self.summary.analysed + self.summary.written + self.summary.not_written
    }

    fn progress(&self, elapsed: Duration) -> BatchProgress {
        let done = self.terminal.iter().filter(|&&t| t).count();
        let total = self.terminal.len();
        let seconds = elapsed.as_secs_f64();
        let realtime_x =
            (self.audio_done > 0.0 && seconds > 0.0).then(|| self.audio_done / seconds);
        let eta = realtime_x.map(|x| {
            // Remaining audio from the hints; files without one count as the average so far.
            // File counts are far below 2^52.
            #[allow(clippy::cast_precision_loss)]
            let average = self.audio_done / self.finished().max(1) as f64;
            let remaining: f64 = self
                .terminal
                .iter()
                .zip(&self.hints)
                .filter(|(done, _)| !**done)
                .map(|(_, hint)| hint.unwrap_or(average))
                .sum();
            Duration::from_secs_f64(remaining / x)
        });
        BatchProgress {
            done,
            total,
            eta,
            realtime_x,
        }
    }
}
