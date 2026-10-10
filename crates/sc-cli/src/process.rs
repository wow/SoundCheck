//! `sc-cli process`: exporting files with the engine's process job (`sc_engine::Task::Process`),
//! the one the app's Export button runs. Crash recovery runs first; every file is analysed (the
//! cache is used), planned as `sc-cli plan --batch-mode` plans it, written in place after a
//! backup (or as a copy with `--out`), verified, and analysed again. Each file prints one line
//! (or one JSON document) in the order given: what was written, or why not; a refusal prints
//! three lines on stderr. A summary line ends the text output.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::ValueEnum;
use sc_core::Error;
use sc_core::export::{
    BatchMode, DEFAULT_LEAD_MS, ExportOutcome, ExportPlan, ExportSettings, Place as ExportPlace,
};
use sc_engine::{
    BatchFile, BatchSettings, CancelToken, EngineEvent, Place, ProcessDone, ProcessSettings,
    RecoveryGate, Task, check_inputs, default_workers, run_batch,
};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;
use sc_io::txn::{TxnKind, TxnReport, hex};
use serde::Serialize;

use crate::apply::{BackupArgs, TXN_SCHEMA, print_failed, recover_first};
use crate::export_plan::{
    BatchModeArg, GRID_WITHHELD, notice_text, parse_lead_ms, skip_text, write_text, xml_only_text,
};
use crate::refusal::Action;
use crate::{AnalysisArgs, ModeArg, decide_settings};

/// Output depth.
#[derive(Clone, Copy, ValueEnum)]
pub enum DepthArg {
    /// The source's (24-bit for a float source).
    Source,
    /// 16-bit, TPDF-dithered.
    #[value(name = "16")]
    Sixteen,
    /// 24-bit.
    #[value(name = "24")]
    TwentyFour,
}

/// The flags of `process`.
#[derive(clap::Args)]
pub struct ProcessArgs {
    /// WAV, AIFF or FLAC files (others are reported as skipped or left to the rekordbox XML).
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Prepare (new tracks: the start may be cut so bar 1 sits a lead after it) or Library
    /// (tracks already in a DJ app: the length never changes).
    #[arg(long, value_enum)]
    batch_mode: BatchModeArg,
    /// Change no audio: no gain and no cut; only tags carry the grid.
    #[arg(long)]
    grid_only: bool,
    /// Write copies into this folder (created if needed) and leave the files as they are.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    /// Output bits per sample.
    #[arg(long, value_enum, default_value = "source")]
    depth: DepthArg,
    /// Time a Prepare cut leaves before bar 1, milliseconds (0 to 50).
    #[arg(long, default_value_t = DEFAULT_LEAD_MS, value_parser = parse_lead_ms)]
    lead_ms: f64,
    /// Do not write the tempo tag (it is written in Prepare mode by default, never in Library).
    #[arg(long)]
    no_tbpm: bool,
    /// Statistic to align: S-P95 for DJ sets, integrated loudness for streaming.
    #[arg(long, value_enum, default_value = "dj")]
    mode: ModeArg,
    /// Target in LUFS (default -11 for dj, -14 for streaming).
    #[arg(long, allow_hyphen_values = true)]
    target: Option<f64>,
    /// True-peak ceiling in dBTP (default -0.5 for dj, -1.0 for streaming).
    #[arg(long, allow_hyphen_values = true)]
    ceiling: Option<f64>,
    /// Print JSON (one document per file) instead of text.
    #[arg(long)]
    json: bool,
    /// Files processed at once (default: a quarter of the logical cores, at most 4).
    #[arg(long)]
    jobs: Option<usize>,
    #[command(flatten)]
    backup: BackupArgs,
    #[command(flatten)]
    analysis: AnalysisArgs,
}

impl ProcessArgs {
    fn export_settings(&self) -> anyhow::Result<ExportSettings> {
        let mode = match self.batch_mode {
            BatchModeArg::Prepare => BatchMode::Prepare,
            BatchModeArg::Library => BatchMode::Library,
        };
        let defaults = ExportSettings::new(mode);
        let settings = ExportSettings {
            place: if self.out.is_some() {
                ExportPlace::Folder
            } else {
                ExportPlace::InPlace
            },
            depth: match self.depth {
                DepthArg::Source => None,
                DepthArg::Sixteen => Some(16),
                DepthArg::TwentyFour => Some(24),
            },
            grid_only: self.grid_only,
            tbpm: defaults.tbpm && !self.no_tbpm,
            lead_ms: self.lead_ms,
            ..defaults
        };
        settings.validate()?;
        Ok(settings)
    }
}

/// What happened to one file.
enum FileResult {
    /// Written: the plan, the transaction and what followed.
    Written {
        plan: Box<ExportPlan>,
        report: Box<TxnReport>,
        done: Box<ProcessDone>,
    },
    /// Not written.
    NotWritten(Box<ExportOutcome>),
    /// Refused or failed.
    Failed(Error),
}

/// Counts for the summary line.
#[derive(Default)]
struct Counts {
    written: usize,
    xml_only: usize,
    skipped: usize,
    failed: usize,
}

/// `sc-cli process`: returns how many files failed.
///
/// # Errors
/// Invalid flags, an unreachable backup, cache or edits folder, or a failed write to stdout.
pub fn run_process(args: &ProcessArgs) -> anyhow::Result<usize> {
    let decide = decide_settings(args.mode, args.target, args.ceiling, &args.analysis);
    decide.validate()?;
    let export = args.export_settings()?;
    let root = args.backup.resolve()?;
    let recovered = recover_first(&root);
    let out_dir = args.out.as_ref().map(std::path::absolute).transpose()?;
    let place = out_dir.clone().map_or(Place::InPlace, Place::Folder);
    let action = if out_dir.is_some() {
        Action::Copy
    } else {
        Action::Change
    };
    let process = ProcessSettings {
        decide,
        export,
        out_dir,
        backup_root: root,
        edits: Some(EditStore::open(EditStore::default_dir()?)),
        recovery: Arc::new(RecoveryGate::new(recovered)),
    };
    let settings = BatchSettings {
        analysis: crate::settings(&args.analysis),
        workers: args.jobs.unwrap_or_else(default_workers),
        cache: Some(Cache::open(Cache::default_dir()?)),
        task: Task::Process(Box::new(process)),
    };
    // Files listed twice and copies that would share a name are refused before any write.
    let mut results: Vec<Option<FileResult>> = check_inputs(&args.files, &place)
        .into_iter()
        .map(|refusal| refusal.map(FileResult::Failed))
        .collect();
    let todo: Vec<usize> = (0..args.files.len())
        .filter(|&i| results[i].is_none())
        .collect();
    let batch: Vec<BatchFile> = todo
        .iter()
        .map(|&i| BatchFile {
            file_id: u32::try_from(i).expect("fewer than 2^32 files on a command line"),
            path: args.files[i].clone(),
            duration_hint: None,
        })
        .collect();
    let mut printer = Printer {
        files: &args.files,
        json: args.json,
        action,
        mode: export.batch_mode,
        next: 0,
        counts: Counts::default(),
        error: None,
    };
    printer.flush(&mut results);
    let mut in_flight = InFlight::default();
    let outcome = run_batch(&batch, &settings, &CancelToken::new(), &mut |event| {
        if let Some((file_id, result)) = in_flight.take(event) {
            if let Some(slot) = results.get_mut(file_id as usize) {
                *slot = Some(result);
            }
            printer.flush(&mut results);
        }
    });
    if let Err(err) = outcome {
        if matches!(err, Error::ModelUnavailable { .. }) {
            crate::model_error_exit(&err);
        }
        return Err(err.into());
    }
    if let Some(err) = printer.error {
        return Err(err);
    }
    let c = &printer.counts;
    if !args.json {
        println!(
            "{} files: {} written, {} XML only, {} skipped, {} failed",
            args.files.len(),
            c.written,
            c.xml_only,
            c.skipped,
            c.failed
        );
    }
    Ok(c.failed)
}

/// What the files being written have reported so far.
#[derive(Default)]
struct InFlight {
    plans: BTreeMap<u32, Box<ExportPlan>>,
    reports: BTreeMap<u32, Box<TxnReport>>,
}

impl InFlight {
    /// Keeps what `event` says about a file being written; returns a file's result once known.
    fn take(&mut self, event: EngineEvent) -> Option<(u32, FileResult)> {
        Some(match event {
            EngineEvent::Processing { file_id, plan } => {
                self.plans.insert(file_id, plan);
                return None;
            }
            EngineEvent::Written { file_id, report } => {
                self.reports.insert(file_id, report);
                return None;
            }
            EngineEvent::Done { file_id, done } => {
                let plan = self.plans.remove(&file_id);
                let report = self.reports.remove(&file_id);
                match (plan, report) {
                    (Some(plan), Some(report)) => {
                        (file_id, FileResult::Written { plan, report, done })
                    }
                    _ => (
                        file_id,
                        FileResult::Failed(Error::Internal(
                            "a written file lost its report".into(),
                        )),
                    ),
                }
            }
            EngineEvent::ExportSkipped {
                file_id, outcome, ..
            } => (file_id, FileResult::NotWritten(outcome)),
            EngineEvent::Failed { file_id, error } => (file_id, FileResult::Failed(error)),
            EngineEvent::Cancelled { file_id } => (file_id, FileResult::Failed(Error::Cancelled)),
            _ => return None,
        })
    }
}

/// Prints results in the order the files were given, as soon as each is known.
struct Printer<'a> {
    files: &'a [PathBuf],
    json: bool,
    action: Action,
    mode: BatchMode,
    next: usize,
    counts: Counts,
    error: Option<anyhow::Error>,
}

impl Printer<'_> {
    fn flush(&mut self, results: &mut [Option<FileResult>]) {
        while let Some(result) = results.get_mut(self.next).and_then(Option::take) {
            let file = &self.files[self.next];
            if let Err(e) = self.print(file, result) {
                self.error.get_or_insert(e);
            }
            self.next += 1;
        }
    }

    fn print(&mut self, file: &Path, result: FileResult) -> anyhow::Result<()> {
        match result {
            FileResult::Failed(err) => {
                self.counts.failed += 1;
                return print_failed(file, err, self.action, self.json);
            }
            FileResult::Written { .. } => self.counts.written += 1,
            FileResult::NotWritten(ref outcome) => match **outcome {
                ExportOutcome::XmlOnly { .. } => self.counts.xml_only += 1,
                _ => self.counts.skipped += 1,
            },
        }
        let mut out = std::io::stdout().lock();
        if self.json {
            if let Some(doc) = doc(file, &result) {
                serde_json::to_writer_pretty(&mut out, &doc)?;
                writeln!(out)?;
            }
        } else {
            write_lines(&mut out, file, &result, self.mode)?;
        }
        Ok(())
    }
}

/// The text of one file that was not refused.
fn write_lines(
    out: &mut impl Write,
    file: &Path,
    result: &FileResult,
    mode: BatchMode,
) -> std::io::Result<()> {
    let head = format!("{}: {}", file.display(), mode.as_str());
    match result {
        FileResult::Written { plan, report, done } => {
            writeln!(
                out,
                "{head}: {}; {}",
                write_text(plan),
                written_text(report, done)
            )?;
            for notice in &plan.notices {
                writeln!(out, "  note: {}", notice_text(*notice))?;
            }
            if plan.grid_withheld {
                writeln!(out, "  note: {GRID_WITHHELD}")?;
            }
            Ok(())
        }
        FileResult::NotWritten(outcome) => match **outcome {
            ExportOutcome::XmlOnly { reason } => {
                writeln!(out, "{head}: XML only: {}", xml_only_text(reason))
            }
            ExportOutcome::Skip { reason } => {
                let (why, what_to_do) = skip_text(reason);
                writeln!(out, "{head}: skipped")?;
                writeln!(out, "  why: {why}")?;
                writeln!(out, "  what to do: {what_to_do}")
            }
            ExportOutcome::Write { ref plan } => writeln!(out, "{head}: {}", write_text(plan)),
        },
        FileResult::Failed(_) => Ok(()),
    }
}

/// `written, verified; backup ...` and what followed the write.
fn written_text(report: &TxnReport, done: &ProcessDone) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("written and verified");
    match (&report.backup, report.kind) {
        (Some(b), _) => {
            let _ = write!(s, "; backup {}", b.display());
        }
        (None, TxnKind::ToFolder) => {
            let _ = write!(s, "; written to {}", done.output.display());
        }
        (None, _) => {}
    }
    if let Some(why) = &report.render.tags_not_added {
        let _ = write!(s, "; tags not added: {why}");
    }
    if done.edit_carried {
        s.push_str(if done.edit.confirmed {
            "; confirmed grid carried over"
        } else {
            "; grid edit carried over"
        });
    }
    for note in report.notes.iter().chain(&done.notes) {
        let _ = write!(s, "; note: {note}");
    }
    s
}

/// The JSON document of one file that was not refused.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessDoc<'a> {
    schema: u32,
    file: String,
    ok: bool,
    /// `write`, `xmlOnly` or `skip`, with the plan or the reason.
    export: ExportOutcome,
    /// The write, for a file written.
    #[serde(skip_serializing_if = "Option::is_none")]
    written: Option<WrittenDoc<'a>>,
}

/// What the write did and what followed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WrittenDoc<'a> {
    txn: &'a str,
    output: String,
    backup: Option<String>,
    sidecar: Option<String>,
    frames_in: u64,
    frames_out: u64,
    trim_frames: u64,
    original_blake3: String,
    output_blake3: String,
    edit_carried: bool,
    grid_confirmed: bool,
    output_analysed: bool,
    notes: Vec<&'a str>,
}

/// `None` for a refusal, which `print_failed` prints.
fn doc<'a>(file: &Path, result: &'a FileResult) -> Option<ProcessDoc<'a>> {
    let (export, written) = match result {
        FileResult::Written { plan, report, done } => (
            ExportOutcome::Write {
                plan: (**plan).clone(),
            },
            Some(WrittenDoc {
                txn: &report.txn,
                output: done.output.display().to_string(),
                backup: report.backup.as_ref().map(|p| p.display().to_string()),
                sidecar: report.sidecar.as_ref().map(|p| p.display().to_string()),
                frames_in: report.render.frames_in,
                frames_out: report.render.frames_out,
                trim_frames: report.render.trim_frames,
                original_blake3: hex(&report.original_blake3),
                output_blake3: hex(&report.output_blake3),
                edit_carried: done.edit_carried,
                grid_confirmed: done.edit.confirmed,
                output_analysed: done.analysis.is_some(),
                notes: report
                    .notes
                    .iter()
                    .chain(&done.notes)
                    .map(String::as_str)
                    .collect(),
            }),
        ),
        FileResult::NotWritten(outcome) => ((**outcome).clone(), None),
        FileResult::Failed(_) => return None,
    };
    Some(ProcessDoc {
        schema: TXN_SCHEMA,
        file: file.display().to_string(),
        ok: true,
        export,
        written,
    })
}
