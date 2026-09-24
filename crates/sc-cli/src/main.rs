//! Headless SoundCheck: the same analysis the app shows, printed as JSON.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use sc_core::AudioSpec;
use serde::Serialize;
use tracing_subscriber::EnvFilter;

/// Version with git revision and build date, e.g. `0.1.0 (a1b2c3d4e, 2026-10-01)`.
const VERSION_LONG: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("SC_GIT_SHA"),
    ", ",
    env!("SC_BUILD_DATE"),
    ")"
);

/// Schema version of the JSON reports; bumped only with a breaking change.
const REPORT_SCHEMA: u32 = 1;

#[derive(Parser)]
#[command(name = "sc-cli", version = VERSION_LONG, about = "Headless SoundCheck")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyse one file and print a report.
    Analyze {
        /// The audio file.
        file: PathBuf,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeReport<'a> {
    schema: u32,
    version: &'static str,
    file: &'a Path,
    spec: AudioSpec,
    frames: usize,
    duration_s: f64,
    sample_peak_dbfs: f64,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Analyze { file, json } => analyze(&file, json),
    }
}

fn analyze(file: &Path, json: bool) -> anyhow::Result<()> {
    let buf = sc_io::read_all(file).with_context(|| format!("analysing {}", file.display()))?;
    let report = AnalyzeReport {
        schema: REPORT_SCHEMA,
        version: sc_core::VERSION,
        file,
        spec: buf.spec,
        frames: buf.frames(),
        duration_s: buf.duration().0,
        sample_peak_dbfs: sc_analysis::sample_peak(&buf.data).0,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", file.display());
        println!(
            "  {} Hz, {} ch, {} frames ({:.3} s)",
            report.spec.sample_rate, report.spec.channels, report.frames, report.duration_s
        );
        println!("  sample peak {:.2} dBFS", report.sample_peak_dbfs);
    }
    Ok(())
}
