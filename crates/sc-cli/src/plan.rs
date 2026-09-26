//! `sc-cli plan`: what processing would do to each file, decided exactly as the app's table
//! decides it (`sc_engine::decide`).

use std::io::Write;
use std::path::{Path, PathBuf};

use sc_core::Error;
use sc_core::analysis::AnalysisRecord;
use sc_core::ipc::{IpcError, JobStage};
use sc_core::plan::{
    Codec, DecideSettings, GainPlan, LoudnessMode, Plan, ReviewReason, SkipReason,
};
use sc_engine::{BatchSettings, REPORT_SCHEMA, decide};
use serde::Serialize;

use crate::report::ErrorReport;

/// The JSON document for one planned file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanReport<'a> {
    schema: u32,
    file: String,
    plan: &'a Plan,
}

/// Row counts, as the app's filter chips show them.
#[derive(Default)]
struct Counts {
    analysed: usize,
    review: usize,
    skipped: usize,
    short: usize,
}

impl Counts {
    fn add(&mut self, plan: &Plan) {
        match plan.status {
            JobStage::NeedsReview => self.review += 1,
            JobStage::Skipped => self.skipped += 1,
            _ => self.analysed += 1,
        }
        if matches!(plan.gain, Some(GainPlan::Gain { short_by_lu, .. }) if short_by_lu > 0.0) {
            self.short += 1;
        }
    }
}

/// Plans every file, printing in the order given; returns how many failed.
pub fn plan_all(
    settings: &BatchSettings,
    decide_settings: &DecideSettings,
    files: &[PathBuf],
    json: bool,
) -> anyhow::Result<usize> {
    let mut failed = 0;
    let mut counts = Counts::default();
    let mut first_error = None;
    crate::run_in_order(settings, files, &mut |file, outcome| {
        let printed = match outcome {
            Ok(report) => {
                let plan = decide(&report.record, Codec::from_path(file), decide_settings);
                counts.add(&plan);
                print_plan(file, &report.record, &plan, decide_settings, json)
            }
            Err(err) => {
                failed += 1;
                print_error(file, err, json)
            }
        };
        if let Err(err) = printed {
            first_error.get_or_insert(err);
        }
    });
    if let Some(err) = first_error {
        return Err(err);
    }
    if !json {
        println!(
            "{} files: {} analysed, {} need review, {} skipped, {failed} failed; {} short of the target",
            files.len(),
            counts.analysed,
            counts.review,
            counts.skipped,
            counts.short
        );
    }
    Ok(failed)
}

fn print_plan(
    file: &Path,
    record: &AnalysisRecord,
    plan: &Plan,
    settings: &DecideSettings,
    json: bool,
) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if json {
        let doc = PlanReport {
            schema: REPORT_SCHEMA,
            file: file.display().to_string(),
            plan,
        };
        serde_json::to_writer_pretty(&mut out, &doc)?;
        writeln!(out)?;
        return Ok(());
    }
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    writeln!(out, "{name}")?;
    let stat = match settings.mode {
        LoudnessMode::Dj => "S-P95",
        LoudnessMode::Streaming => "I",
    };
    let measured = plan
        .measured
        .map_or_else(|| "n/a".to_owned(), |m| format!("{:.1}", m.0));
    write!(
        out,
        "  {stat} {measured} -> {:.1} LUFS  {}  {}",
        settings.target.0,
        action(plan),
        status(plan.status)
    )?;
    if !plan.review.is_empty() {
        let reasons: Vec<String> = plan
            .review
            .iter()
            .map(|r| review(r, record, settings))
            .collect();
        write!(out, ": {}", reasons.join(", "))?;
    }
    writeln!(out)?;
    Ok(())
}

fn print_error(file: &Path, err: Error, json: bool) -> anyhow::Result<()> {
    eprintln!("sc-cli: {err}");
    if json {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let doc = ErrorReport {
            schema: REPORT_SCHEMA,
            file: file.display().to_string(),
            error: IpcError::from(err),
        };
        serde_json::to_writer_pretty(&mut out, &doc)?;
        writeln!(out)?;
    }
    Ok(())
}

/// The Action column's first line.
fn action(plan: &Plan) -> String {
    if let Some(skip) = plan.skip {
        return match skip {
            SkipReason::AnalyseOnly { codec } => format!("Skip: {} is analyse-only", codec.label()),
            SkipReason::Silent => "Skip: silent".to_owned(),
        };
    }
    match plan.gain {
        Some(GainPlan::Gain {
            gain_db,
            short_by_lu,
            ..
        }) if short_by_lu > 0.0 => format!("Gain {gain_db:+.1} dB, short by {short_by_lu:.1} LU"),
        Some(GainPlan::Gain { gain_db, .. }) => format!("Gain {gain_db:+.1} dB"),
        Some(GainPlan::GlobalGain {
            steps,
            gain_db,
            residual_lu,
            ..
        }) => {
            format!("Global-gain {gain_db:+.1} dB ({steps:+} steps), residual {residual_lu:+.1} LU")
        }
        Some(GainPlan::AtTarget) => "Already at target".to_owned(),
        None => "-".to_owned(),
    }
}

fn status(stage: JobStage) -> &'static str {
    match stage {
        JobStage::NeedsReview => "Needs review",
        JobStage::Skipped => "Skipped",
        _ => "Analysed",
    }
}

fn review(reason: &ReviewReason, record: &AnalysisRecord, settings: &DecideSettings) -> String {
    match reason {
        ReviewReason::Confidence => {
            let why: Vec<String> = record
                .grid
                .iter()
                .flat_map(|g| g.reasons.iter().map(|r| format!("{r:?}")))
                .collect();
            format!("low confidence ({})", why.join(", "))
        }
        ReviewReason::CheckGrid => "check grid".to_owned(),
        ReviewReason::Drifts => "drifts".to_owned(),
        ReviewReason::OutsideBpmRange => format!(
            "BPM outside {:.0}-{:.0}",
            settings.bpm_range.0.0, settings.bpm_range.1.0
        ),
        ReviewReason::TagBpmDisagrees { tag } => format!("tag says {:.2}", tag.0),
        ReviewReason::NoGrid => "no beats found".to_owned(),
    }
}
