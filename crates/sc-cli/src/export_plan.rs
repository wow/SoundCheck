//! `sc-cli plan --batch-mode ...`: what exporting would do to each file (`sc_engine::plan_export`),
//! printed after the file's plan: the gain and the cut, the file left to the rekordbox XML, or
//! the reason it is skipped in three lines (what, why, what to do).

use std::io::Write;

use clap::ValueEnum;
use sc_core::export::{
    BatchMode, Cut, DEFAULT_LEAD_MS, ExportOutcome, ExportPlan, ExportSettings, ExportSkip,
    LEAD_MS_RANGE, Place,
};
pub(crate) use sc_engine::export::wording::{GRID_WITHHELD, notice_text, skip_why, xml_only_text};

#[derive(Clone, Copy, ValueEnum)]
pub enum BatchModeArg {
    /// New tracks: lossless files may lose up to one beat at the start so bar 1 sits a lead
    /// after it.
    Prepare,
    /// Tracks already in a DJ app: the length never changes.
    Library,
}

/// The export flags of `plan`.
#[derive(clap::Args)]
pub struct ExportArgs {
    /// Also print what exporting would do to each file in this batch mode.
    #[arg(long, value_enum)]
    batch_mode: Option<BatchModeArg>,
    /// Change no audio: no gain and no cut; only tags and the XML carry the grid.
    #[arg(long, requires = "batch_mode")]
    grid_only: bool,
    /// Time a Prepare cut leaves before bar 1, milliseconds (2, the fade after the cut, to 50).
    #[arg(long, requires = "batch_mode", default_value_t = DEFAULT_LEAD_MS, value_parser = parse_lead_ms)]
    lead_ms: f64,
    /// No rekordbox XML for the batch: a file only the XML could carry has nothing to write.
    #[arg(long, requires = "batch_mode")]
    no_xml: bool,
}

pub(crate) fn parse_lead_ms(text: &str) -> Result<f64, String> {
    let ms: f64 = text
        .parse()
        .map_err(|e| format!("{text:?} is not a number: {e}"))?;
    let (lo, hi) = LEAD_MS_RANGE;
    if ms.is_finite() && (lo..=hi).contains(&ms) {
        Ok(ms)
    } else {
        Err(format!("{ms} ms is outside {lo} to {hi} ms"))
    }
}

impl ExportArgs {
    /// The export settings the flags describe, or `None` without `--batch-mode`.
    ///
    /// # Errors
    /// The settings' own check (`--lead-ms` out of range).
    pub fn settings(&self) -> anyhow::Result<Option<ExportSettings>> {
        let Some(mode) = self.batch_mode else {
            return Ok(None);
        };
        // What an export in place would do: the strictest case for Serato data and tags.
        let settings = ExportSettings {
            place: Place::InPlace,
            grid_only: self.grid_only,
            lead_ms: self.lead_ms,
            xml: !self.no_xml,
            ..ExportSettings::new(match mode {
                BatchModeArg::Prepare => BatchMode::Prepare,
                BatchModeArg::Library => BatchMode::Library,
            })
        };
        settings.validate()?;
        Ok(Some(settings))
    }
}

/// Writes the export lines of one file.
///
/// # Errors
/// Whatever `out` returns.
pub fn write_outcome(
    out: &mut impl Write,
    outcome: &ExportOutcome,
    mode: BatchMode,
) -> std::io::Result<()> {
    let head = format!("  Export ({}):", mode.as_str());
    match outcome {
        ExportOutcome::Write { plan } => {
            writeln!(out, "{head} {}", write_text(plan))?;
            for notice in &plan.notices {
                writeln!(out, "    note: {}", notice_text(*notice))?;
            }
            if plan.grid_withheld {
                writeln!(out, "    note: {GRID_WITHHELD}")?;
            }
            Ok(())
        }
        ExportOutcome::XmlOnly { reason } => {
            writeln!(out, "{head} XML only: {}", xml_only_text(*reason))
        }
        ExportOutcome::Skip { reason } => {
            let (why, what_to_do) = skip_text(*reason);
            writeln!(out, "{head} skipped")?;
            writeln!(out, "    why: {why}")?;
            writeln!(out, "    what to do: {what_to_do}")
        }
    }
}

/// `Gain -2.0 dB, Cut 0.29 s; tags BPM, ...`.
pub(crate) fn write_text(plan: &ExportPlan) -> String {
    let gain = if plan.gain_db.abs() < sc_core::plan::NEGLIGIBLE_DB {
        "No gain".to_owned()
    } else {
        format!("Gain {:+.1} dB", plan.gain_db)
    };
    let head = match plan.cut {
        Cut::Cut { seconds, .. } => format!("{gain}, Cut {:.2} s", seconds.0),
        Cut::OnBar {
            bar_line,
            bar_line_s,
            bar1,
            bar1_s,
        } => {
            // "bar 1" only when the line is bar 1 itself, else the line and where bar 1 is.
            let (line, anchor, sep) = if bar_line == bar1 {
                ("bar 1", String::new(), ",")
            } else {
                ("a bar line", format!(" (bar 1 at {:.2} s)", bar1_s.0), "")
            };
            let at = if bar_line.0 == 0 {
                String::new()
            } else {
                format!("{sep} {:.3} s in", bar_line_s.0)
            };
            format!("{gain}, Starts on {line}{at}{anchor}")
        }
        Cut::NotCut {
            bar1,
            bar1_s,
            first_bar_line,
            first_bar_line_s,
        } => {
            if bar1 == first_bar_line {
                format!("{gain}, Not cut: bar 1 {:.2} s in", bar1_s.0)
            } else {
                format!(
                    "{gain}, Not cut: first bar line {:.2} s in (bar 1 at {:.2} s)",
                    first_bar_line_s.0, bar1_s.0
                )
            }
        }
        Cut::NeedsReview { bar1_s, .. } => format!(
            "{gain}, Not cut: grid needs review (bar 1 at {:.2} s; confirm it in the app)",
            bar1_s.0
        ),
        Cut::NoGrid => format!("{gain}, Not cut: no grid"),
        Cut::GridOnly => "Grid only (no audio change)".to_owned(),
        Cut::Library => format!("{gain}, length kept"),
    };
    let depth = plan.bits.map_or_else(String::new, |b| format!(", {b}-bit"));
    let names: Vec<&str> = plan.tags.iter().map(|t| t.name.as_str()).collect();
    format!("{head}{depth}; tags {}", names.join(", "))
}

/// Why a file is skipped, and what to do with this command line.
pub(crate) fn skip_text(reason: ExportSkip) -> (String, String) {
    (skip_why(reason), what_to_do(reason).to_owned())
}

/// What to do about a skipped file, in this command line's flags.
fn what_to_do(reason: ExportSkip) -> &'static str {
    match reason {
        ExportSkip::SeratoInPlaceCut { .. } => {
            "export it to a folder (the original keeps its cues), or use --batch-mode library"
        }
        ExportSkip::SeratoUnknownInPlaceCut { .. } => {
            "export it to a folder (the original stays as it is), or use --batch-mode library"
        }
        ExportSkip::NotDjSafeRate { .. } => "convert it to 44.1 or 48 kHz in an audio editor first",
        ExportSkip::UnsupportedChannels { .. } => "export a stereo version first",
        ExportSkip::NothingToWrite { .. } => "run it again without --no-xml",
        ExportSkip::Unsupported { .. } => "convert it to AIFF, WAV or FLAC first",
        ExportSkip::Silent => "check that it is the right file",
        ExportSkip::NoGrid => "set the grid in the app, or export without --grid-only",
        ExportSkip::XmlNoGrid { .. } => "set the grid in the app first",
        ExportSkip::XmlGridNeedsReview { .. } => "check and confirm the grid in the app",
        ExportSkip::XmlNotOptedIn { .. } => {
            "confirm the grid in the app to include it, or use --batch-mode prepare"
        }
        ExportSkip::GridNeedsReview => {
            "check and confirm the grid in the app, or export without --grid-only"
        }
    }
}

#[cfg(test)]
mod tests;
