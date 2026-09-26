//! Headless SoundCheck: the same analysis the app shows, printed as text or JSON.

mod eval;
mod plan;
mod report;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use sc_core::analysis::{AnalysisSettings, Model};
use sc_core::ipc::IpcError;
use sc_core::plan::DecideSettings;
use sc_core::{Bpm, DbTp, Error, Lufs};
use sc_engine::{
    AnalyzeReport, Analyzer, BatchFile, BatchSettings, CancelToken, EngineEvent, REPORT_SCHEMA,
    Timings, default_workers, run_batch,
};
use sc_io::cache::Cache;
use tracing_subscriber::EnvFilter;

use crate::report::{ErrorReport, write_text};

/// Version with git revision and build date, e.g. `0.1.0 (a1b2c3d4e, 2026-10-01)`.
const VERSION_LONG: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("SC_GIT_SHA"),
    ", ",
    env!("SC_BUILD_DATE"),
    ")"
);

/// Exit code when at least one file failed, or the analysis could not start.
const EXIT_FAILED: i32 = 2;

#[derive(Parser)]
#[command(name = "sc-cli", version = VERSION_LONG, about = "Headless SoundCheck")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModelArg {
    /// The bundled small model.
    Small,
    /// The full model (developer option; `scripts/fetch-models.sh --full`).
    Full,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    /// S-P95, the DJ alignment statistic.
    Dj,
    /// Integrated loudness, for streaming platforms.
    Streaming,
}

#[derive(clap::Args)]
struct AnalysisArgs {
    /// Loudness only: skip beat tracking and the grid.
    #[arg(long)]
    no_grid: bool,
    /// The DJ app's BPM range; the tempo octave is chosen inside it first.
    #[arg(long, default_value = "70-180", value_parser = parse_bpm_range)]
    bpm_range: (f64, f64),
    /// Beat-tracking model.
    #[arg(long, value_enum, default_value = "small")]
    model: ModelArg,
}

#[derive(Subcommand)]
enum Command {
    /// Analyse files and print a report per file.
    Analyze {
        /// Audio files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Print JSON (one document per file) instead of text.
        #[arg(long)]
        json: bool,
        /// Include the grid evidence (model beats, activations, onsets) in the JSON.
        #[arg(long)]
        evidence: bool,
        /// Neither read nor write the analysis cache.
        #[arg(long)]
        no_cache: bool,
        /// Files analysed at once (default: a quarter of the logical cores, at most 4). Reports
        /// are printed in the order the files were given.
        #[arg(long)]
        jobs: Option<usize>,
        #[command(flatten)]
        analysis: AnalysisArgs,
    },
    /// Analyse files (using the cache) and print what processing would do to each: the gain,
    /// what would be skipped, and what needs a look first. The app decides every row the same way.
    Plan {
        /// Audio files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
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
        /// Files analysed at once (default: a quarter of the logical cores, at most 4).
        #[arg(long)]
        jobs: Option<usize>,
        #[command(flatten)]
        analysis: AnalysisArgs,
    },
    /// Time each analysis stage on one file (cache off) and print real-time factors.
    Bench {
        /// The audio file.
        file: PathBuf,
        /// Runs; the median of each stage is reported.
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[command(flatten)]
        analysis: AnalysisArgs,
    },
    /// Score the analysis against hand labels (CSV columns `file`, `bpm`, `bar1_s`, `meter`,
    /// `grouping`, `ffmpeg_i_lufs`); files are found by name anywhere under the folder. The cache
    /// is not used.
    Eval {
        /// The labels CSV.
        #[arg(long)]
        labels: PathBuf,
        /// Folder holding the labelled files.
        #[arg(long)]
        dir: PathBuf,
        /// Print JSON (rows and summary) instead of text.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        analysis: AnalysisArgs,
    },
    /// Inspect or empty the analysis cache.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand, Clone, Copy)]
enum CacheAction {
    /// Print the cache directory.
    Path,
    /// Remove every cached analysis.
    Clear,
}

fn parse_bpm_range(text: &str) -> Result<(f64, f64), String> {
    let (lo, hi) = text
        .split_once('-')
        .ok_or_else(|| "expected LOW-HIGH, e.g. 70-180".to_string())?;
    let parse = |s: &str| {
        s.trim()
            .parse::<f64>()
            .map_err(|e| format!("{s:?} is not a number: {e}"))
    };
    let (lo, hi) = (parse(lo)?, parse(hi)?);
    if !(lo > 0.0 && hi > lo && hi <= 1000.0) {
        return Err(format!(
            "range {lo}-{hi} must satisfy 0 < LOW < HIGH <= 1000"
        ));
    }
    Ok((lo, hi))
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Analyze {
            files,
            json,
            evidence,
            no_cache,
            jobs,
            analysis,
        } => {
            let cache = if no_cache {
                None
            } else {
                Some(Cache::open(Cache::default_dir()?))
            };
            let settings = BatchSettings {
                analysis: settings(&analysis),
                workers: jobs.unwrap_or_else(default_workers),
                cache,
            };
            let failed = analyze_all(&settings, &files, json, evidence)?;
            if failed > 0 {
                std::process::exit(EXIT_FAILED);
            }
            Ok(())
        }
        Command::Plan {
            files,
            mode,
            target,
            ceiling,
            json,
            jobs,
            analysis,
        } => {
            let mut decide = match mode {
                ModeArg::Dj => DecideSettings::dj(),
                ModeArg::Streaming => DecideSettings::streaming(),
            };
            if let Some(t) = target {
                decide.target = Lufs(t);
            }
            if let Some(c) = ceiling {
                decide.ceiling = DbTp(c);
            }
            decide.bpm_range = (Bpm(analysis.bpm_range.0), Bpm(analysis.bpm_range.1));
            let settings = BatchSettings {
                analysis: settings(&analysis),
                workers: jobs.unwrap_or_else(default_workers),
                cache: Some(Cache::open(Cache::default_dir()?)),
            };
            let failed = plan::plan_all(&settings, &decide, &files, json)?;
            if failed > 0 {
                std::process::exit(EXIT_FAILED);
            }
            Ok(())
        }
        Command::Bench {
            file,
            runs,
            analysis,
        } => {
            let mut analyzer = analyzer(&analysis, None);
            bench(&mut analyzer, &file, runs.max(1))
        }
        Command::Eval {
            labels,
            dir,
            json,
            analysis,
        } => {
            let text = std::fs::read_to_string(&labels)
                .with_context(|| format!("reading {}", labels.display()))?;
            let labels = eval::parse_labels(&text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", labels.display()))?;
            let mut analyzer = analyzer(&analysis, None);
            let (rows, summary) = eval::run(&mut analyzer, &labels, &dir);
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            if json {
                serde_json::to_writer_pretty(
                    &mut out,
                    &serde_json::json!({ "rows": rows, "summary": summary }),
                )?;
                writeln!(out)?;
            } else {
                eval::write_text(&rows, &summary, &mut out)?;
            }
            Ok(())
        }
        Command::Cache { action } => cache_command(action),
    }
}

/// The analysis settings the flags describe.
fn settings(args: &AnalysisArgs) -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(args.bpm_range.0), Bpm(args.bpm_range.1)),
        grid: !args.no_grid,
        model: match args.model {
            ModelArg::Small => Model::Small,
            ModelArg::Full => Model::Full,
        },
    }
}

/// Stops the run when the model cannot be loaded, saying what to do.
fn model_error_exit(err: &Error) -> ! {
    eprintln!(
        "sc-cli: {err}\nrun scripts/fetch-models.sh (or set SC_MODEL_DIR), or pass --no-grid for loudness only"
    );
    std::process::exit(EXIT_FAILED);
}

/// Builds one analyzer (bench, eval); a missing model stops the run before any file.
fn analyzer(args: &AnalysisArgs, cache: Option<Cache>) -> Analyzer {
    Analyzer::load(settings(args), cache, CancelToken::new())
        .unwrap_or_else(|err| model_error_exit(&err))
}

/// Analyses every file on the engine's workers, printing reports in the order the files were
/// given; returns how many failed.
fn analyze_all(
    settings: &BatchSettings,
    files: &[PathBuf],
    json: bool,
    evidence: bool,
) -> anyhow::Result<usize> {
    let mut failed = 0;
    let mut first_error = None;
    run_in_order(settings, files, &mut |file, outcome| {
        let printed = print_report(file, outcome, json, evidence, &mut failed);
        if let Err(err) = printed {
            first_error.get_or_insert(err);
        }
    });
    first_error.map_or(Ok(failed), Err)
}

fn print_report(
    file: &Path,
    outcome: Result<AnalyzeReport, Error>,
    json: bool,
    evidence: bool,
    failed: &mut usize,
) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match outcome {
        Ok(mut report) => {
            if json {
                if !evidence {
                    report.record.evidence = None;
                }
                serde_json::to_writer_pretty(&mut out, &report)?;
                writeln!(out)?;
            } else {
                write_text(&report, &mut out)?;
            }
        }
        Err(err) => {
            *failed += 1;
            eprintln!("sc-cli: {err}");
            if json {
                let report = ErrorReport {
                    schema: REPORT_SCHEMA,
                    file: file.display().to_string(),
                    error: IpcError::from(err),
                };
                serde_json::to_writer_pretty(&mut out, &report)?;
                writeln!(out)?;
            }
        }
    }
    Ok(())
}

/// Runs `files` through the engine and hands each outcome to `emit` in the order the files were
/// given; a model that cannot be loaded stops the run before any file.
fn run_in_order(
    settings: &BatchSettings,
    files: &[PathBuf],
    emit: &mut dyn FnMut(&Path, Result<AnalyzeReport, Error>),
) {
    let batch: Vec<BatchFile> = files
        .iter()
        .enumerate()
        .map(|(i, path)| BatchFile {
            file_id: u32::try_from(i).expect("fewer than 2^32 files on a command line"),
            path: path.clone(),
            duration_hint: None,
        })
        .collect();
    let mut ready: Vec<Option<Result<AnalyzeReport, Error>>> =
        (0..files.len()).map(|_| None).collect();
    let mut next = 0;
    let result = run_batch(&batch, settings, &CancelToken::new(), &mut |event| {
        let (file_id, outcome) = match event {
            EngineEvent::Analysed { file_id, report } => (file_id, Ok(*report)),
            EngineEvent::Failed { file_id, error } => (file_id, Err(error)),
            EngineEvent::Cancelled { file_id } => (file_id, Err(Error::Cancelled)),
            _ => return,
        };
        if let Some(slot) = ready.get_mut(file_id as usize) {
            *slot = Some(outcome);
        }
        while let Some(outcome) = ready.get_mut(next).and_then(Option::take) {
            emit(&files[next], outcome);
            next += 1;
        }
    });
    if let Err(err) = result {
        model_error_exit(&err);
    }
}

/// Picks one stage's time out of [`Timings`].
type Stage = fn(&Timings) -> Duration;

fn bench(analyzer: &mut Analyzer, file: &Path, runs: usize) -> anyhow::Result<()> {
    let mut all: Vec<Timings> = Vec::with_capacity(runs);
    let mut duration = 0.0;
    for _ in 0..runs {
        let mut t = Timings::default();
        let report = analyzer
            .analyze_timed(file, &mut t)
            .with_context(|| format!("analysing {}", file.display()))?;
        duration = report.record.duration.0;
        all.push(t);
    }
    let median = |pick: fn(&Timings) -> Duration| {
        let mut v: Vec<f64> = all.iter().map(|t| pick(t).as_secs_f64()).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    println!("{} ({duration:.1} s, median of {runs})", file.display());
    let stages: [(&str, Stage); 6] = [
        ("decode", |t| t.decode),
        ("loudness", |t| t.loudness),
        ("resample", |t| t.resample),
        ("beats", |t| t.beats),
        ("onsets", |t| t.onsets),
        ("meter + grid", |t| t.grid),
    ];
    let mut total = 0.0;
    for (name, pick) in stages {
        let s = median(pick);
        total += s;
        println!("  {name:<13} {:>8.3} s  {}", s, real_time(duration, s));
    }
    println!(
        "  {:<13} {:>8.3} s  {}",
        "analyze",
        total,
        real_time(duration, total)
    );
    Ok(())
}

fn real_time(duration: f64, seconds: f64) -> String {
    if seconds > 0.0 {
        format!("{:>7.1}x real time", duration / seconds)
    } else {
        "        (skipped)".to_owned()
    }
}

fn cache_command(action: CacheAction) -> anyhow::Result<()> {
    let cache = Cache::open(Cache::default_dir()?);
    match action {
        CacheAction::Path => println!("{}", cache.dir().display()),
        CacheAction::Clear => {
            let removed = cache
                .clear()
                .with_context(|| format!("clearing {}", cache.dir().display()))?;
            println!(
                "removed {removed} cached analyses from {}",
                cache.dir().display()
            );
        }
    }
    Ok(())
}
