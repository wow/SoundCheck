//! Headless SoundCheck: the same analysis the app shows, printed as text or JSON.

mod analyze;

use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use sc_core::Bpm;
use sc_core::analysis::AnalysisSettings;
use sc_core::ipc::IpcError;
use sc_io::cache::Cache;
use tracing_subscriber::EnvFilter;

use crate::analyze::{ErrorReport, Options, REPORT_SCHEMA, analyze_file, write_text};

/// Version with git revision and build date, e.g. `0.1.0 (a1b2c3d4e, 2026-10-01)`.
const VERSION_LONG: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("SC_GIT_SHA"),
    ", ",
    env!("SC_BUILD_DATE"),
    ")"
);

/// Exit code when at least one file failed.
const EXIT_FILE_FAILED: i32 = 2;

#[derive(Parser)]
#[command(name = "sc-cli", version = VERSION_LONG, about = "Headless SoundCheck")]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
        /// Loudness only: skip beat tracking and the grid.
        #[arg(long)]
        no_grid: bool,
        /// The DJ app's BPM range; the tempo octave is chosen inside it first.
        #[arg(long, default_value = "70-180", value_parser = parse_bpm_range)]
        bpm_range: (f64, f64),
        /// Neither read nor write the analysis cache.
        #[arg(long)]
        no_cache: bool,
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
            no_grid,
            bpm_range,
            no_cache,
        } => {
            let settings = AnalysisSettings {
                bpm_range: (Bpm(bpm_range.0), Bpm(bpm_range.1)),
                grid: !no_grid,
                ..AnalysisSettings::default()
            };
            let cache = if no_cache {
                None
            } else {
                Some(Cache::open(Cache::default_dir()?))
            };
            let opts = Options { settings, cache };
            let failed = analyze_all(&files, &opts, json)?;
            if failed > 0 {
                std::process::exit(EXIT_FILE_FAILED);
            }
            Ok(())
        }
        Command::Cache { action } => cache_command(action),
    }
}

/// Analyses every file, printing as it goes; returns how many failed.
fn analyze_all(files: &[PathBuf], opts: &Options, json: bool) -> anyhow::Result<usize> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut failed = 0;
    for file in files {
        match analyze_file(file, opts) {
            Ok(report) => {
                if json {
                    serde_json::to_writer_pretty(&mut out, &report)?;
                    writeln!(out)?;
                } else {
                    write_text(&report, &mut out)?;
                }
            }
            Err(err) => {
                failed += 1;
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
    }
    Ok(failed)
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
