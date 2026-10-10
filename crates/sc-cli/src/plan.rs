//! `sc-cli plan`: what processing would do to each file, decided exactly as the app's table
//! decides it (`sc_engine::decide`), with the grid edits saved in the app applied; with
//! `--batch-mode`, also what exporting would do (see `export_plan`).

use std::io::Write;
use std::path::{Path, PathBuf};

use sc_core::Error;
use sc_core::analysis::AnalysisRecord;
use sc_core::export::{BatchMode, ExportOutcome, ExportSettings};
use sc_core::ipc::{IpcError, JobStage};
use sc_core::plan::{
    Codec, DecideSettings, GainPlan, LoudnessMode, Plan, ReviewReason, SkipReason,
};
use sc_engine::{
    BatchSettings, EditState, ExportInput, ExportSource, REPORT_SCHEMA, apply_saved, decide,
    plan_export_snapped,
};
use sc_io::edits::EditStore;
use serde::Serialize;

use crate::export_plan::ExportArgs;
use crate::report::ErrorReport;
use crate::{AnalysisArgs, ModeArg, cached_batch, decide_settings};

/// The flags of `plan`.
#[derive(clap::Args)]
pub struct PlanArgs {
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
    export: ExportArgs,
    #[command(flatten)]
    analysis: AnalysisArgs,
}

/// Runs `plan`; returns how many files failed.
///
/// # Errors
/// Invalid export flags, an unreachable cache or edits folder, or a failed write to stdout.
pub fn run_plan(args: &PlanArgs) -> anyhow::Result<usize> {
    let decide = decide_settings(args.mode, args.target, args.ceiling, &args.analysis);
    let export = args.export.settings()?;
    let edits = EditStore::open(EditStore::default_dir()?);
    let batch = cached_batch(&args.analysis, args.jobs)?;
    plan_all(
        &batch,
        &decide,
        export.as_ref(),
        Some(&edits),
        &args.files,
        args.json,
    )
}

/// The JSON document for one planned file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanReport<'a> {
    schema: u32,
    file: String,
    plan: &'a Plan,
    /// The grid is the user's edit of the analysed one.
    grid_edited: bool,
    /// The user confirmed the grid in the app.
    grid_confirmed: bool,
    /// What exporting would do, with `--batch-mode`.
    #[serde(skip_serializing_if = "Option::is_none")]
    export: Option<&'a ExportOutcome>,
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

/// Plans every file with the edits saved in `edits` (and, with `export`, its export), printing
/// in the order given; returns how many failed.
pub fn plan_all(
    settings: &BatchSettings,
    decide_settings: &DecideSettings,
    export: Option<&ExportSettings>,
    edits: Option<&EditStore>,
    files: &[PathBuf],
    json: bool,
) -> anyhow::Result<usize> {
    let mut failed = 0;
    let mut counts = Counts::default();
    let mut first_error = None;
    crate::run_in_order(settings, files, &mut |file, outcome| {
        let printed = match outcome {
            Ok(mut report) => {
                let edit = edits
                    .map(|store| apply_saved(&mut report.record, store))
                    .unwrap_or_default();
                let codec = Codec::from_path(file);
                let plan = decide(&report.record, codec, decide_settings, edit.confirmed);
                let export = export.map(|s| {
                    // Read now, not when the file was added: a tag that failed to read then
                    // (Serato data not ruled out) is read again.
                    let source = ExportSource::read(file);
                    let input = ExportInput {
                        record: &report.record,
                        plan: &plan,
                        decide: decide_settings,
                        source: &source,
                    };
                    plan_export_snapped(file, &input, s).map(|outcome| (outcome, s.batch_mode))
                });
                match export.transpose() {
                    Ok(export) => {
                        // Counted only once planned: a file whose cut could not be read is
                        // counted as failed, not also as analysed.
                        counts.add(&plan);
                        let printed = Printed {
                            record: &report.record,
                            plan: &plan,
                            edit,
                            export: export.as_ref(),
                        };
                        print_plan(file, &printed, decide_settings, json)
                    }
                    Err(err) => {
                        failed += 1;
                        print_error(file, err, json)
                    }
                }
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

/// What is printed for one planned file.
struct Printed<'a> {
    record: &'a AnalysisRecord,
    plan: &'a Plan,
    edit: EditState,
    export: Option<&'a (ExportOutcome, BatchMode)>,
}

fn print_plan(
    file: &Path,
    printed: &Printed<'_>,
    settings: &DecideSettings,
    json: bool,
) -> anyhow::Result<()> {
    let Printed {
        record,
        plan,
        edit,
        export,
    } = *printed;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if json {
        let doc = PlanReport {
            schema: REPORT_SCHEMA,
            file: file.display().to_string(),
            plan,
            grid_edited: edit.edited,
            grid_confirmed: edit.confirmed,
            export: export.map(|(outcome, _)| outcome),
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
    match (edit.edited, edit.confirmed) {
        (true, true) => write!(out, "  (grid edited and confirmed)")?,
        (true, false) => write!(out, "  (grid edited)")?,
        (false, true) => write!(out, "  (grid confirmed)")?,
        (false, false) => {}
    }
    writeln!(out)?;
    if let Some((outcome, mode)) = export {
        crate::export_plan::write_outcome(&mut out, outcome, *mode)?;
    }
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
        ReviewReason::Drifts => "drifts".to_owned(),
        ReviewReason::OutsideBpmRange => format!(
            "BPM outside {:.0}-{:.0}",
            settings.bpm_range.0.0, settings.bpm_range.1.0
        ),
        ReviewReason::TagBpmDisagrees { tag } => format!("tag says {:.2}", tag.0),
        ReviewReason::NoGrid => "no beats found".to_owned(),
    }
}
