//! `sc-cli labels`: evaluation labels from the grids the app shows. Confirming a grid by ear in
//! the app and printing it here is how the evaluation set is labelled; `sc-cli eval` reads the
//! same columns. `fit` says which part of the track the grid was fitted to (`whole`, or `start`
//! for a track whose tempo changes), so the label is scored against the same kind of fit.

use std::io::Write;
use std::path::PathBuf;

use sc_core::analysis::{AnalysisRecord, GridFit};
use sc_engine::{BatchSettings, EditState, apply_saved};
use sc_io::edits::EditStore;

/// Prints one CSV row per file with a grid (a comment line for the others); returns how many
/// files failed to analyse.
pub fn labels_all(
    settings: &BatchSettings,
    edits: &EditStore,
    files: &[PathBuf],
    confirmed_only: bool,
) -> anyhow::Result<usize> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "file,bpm,bar1_s,meter,grouping,confirmed,fit")?;
    let mut failed = 0;
    let mut first_error = None;
    crate::run_in_order(settings, files, &mut |file, outcome| {
        let name = file.file_name().map_or_else(
            || file.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let line = match outcome {
            Ok(mut report) => {
                let edit = apply_saved(&mut report.record, edits);
                row(&name, &report.record, edit, confirmed_only)
            }
            Err(err) => {
                failed += 1;
                eprintln!("sc-cli: {err}");
                format!("# {}: {err}", csv(&name))
            }
        };
        if let Err(err) = writeln!(out, "{line}") {
            first_error.get_or_insert(err);
        }
    });
    first_error.map_or(Ok(failed), |e| Err(e.into()))
}

/// The CSV line (or comment) for one analysed file.
fn row(name: &str, record: &AnalysisRecord, edit: EditState, confirmed_only: bool) -> String {
    let Some(grid) = &record.grid else {
        let why = record.grid_skipped.as_deref().unwrap_or("no grid");
        return format!("# {}: {why}", csv(name));
    };
    if confirmed_only && !edit.confirmed {
        return format!("# {}: not confirmed", csv(name));
    }
    let meter = grid.meter.to_string();
    let meter = meter.split(" · ").next().unwrap_or(&meter).to_owned();
    let grouping = if grid.meter.grouping.iter().any(|&g| g > 1) {
        grid.meter
            .grouping
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join("+")
    } else {
        String::new()
    };
    format!(
        "{},{:.2},{:.3},{meter},{grouping},{},{}",
        csv(name),
        grid.bpm.0,
        grid.anchor.to_seconds(record.spec.sample_rate).0,
        if edit.confirmed { "yes" } else { "no" },
        fit_name(edit.fit)
    )
}

/// The `fit` cell: `whole` or `start`.
pub fn fit_name(fit: GridFit) -> &'static str {
    match fit {
        GridFit::Whole => "whole",
        GridFit::Start => "start",
    }
}

/// A CSV cell, quoted when it holds a comma, a quote or a line break, or starts with `#` (which
/// would read as a comment).
fn csv(cell: &str) -> String {
    if cell.contains([',', '"', '\n', '\r']) || cell.starts_with('#') {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_owned()
    }
}

#[cfg(test)]
mod tests;
