//! `sc-cli plan --batch-mode ...`: what exporting would do to each file (`sc_engine::plan_export`),
//! printed after the file's plan: the gain and the cut, the file left to the rekordbox XML, or
//! the reason it is skipped in three lines (what, why, what to do).

use std::io::Write;

use clap::ValueEnum;
use sc_core::export::{
    BatchMode, Cut, DEFAULT_LEAD_MS, ExportNotice, ExportOutcome, ExportPlan, ExportSettings,
    ExportSkip, LEAD_MS_RANGE, XmlOnlyReason,
};

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
    /// Time a Prepare cut leaves before bar 1, milliseconds (0 to 50).
    #[arg(long, requires = "batch_mode", default_value_t = DEFAULT_LEAD_MS, value_parser = parse_lead_ms)]
    lead_ms: f64,
    /// No rekordbox XML for the batch: a file only the XML could carry has nothing to write.
    #[arg(long, requires = "batch_mode")]
    no_xml: bool,
}

fn parse_lead_ms(text: &str) -> Result<f64, String> {
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
        let settings = ExportSettings {
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
fn write_text(plan: &ExportPlan) -> String {
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
        } => {
            if bar_line.0 == 0 {
                format!("{gain}, Starts on bar 1")
            } else {
                format!("{gain}, Starts on a bar line {:.3} s in", bar_line_s.0)
            }
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
        Cut::NoGrid => format!("{gain}, Not cut: no grid"),
        Cut::GridOnly => "Grid only (no audio change)".to_owned(),
        Cut::Library => format!("{gain}, length kept"),
    };
    let depth = plan.bits.map_or_else(String::new, |b| format!(", {b}-bit"));
    let names: Vec<&str> = plan.tags.iter().map(|t| t.name.as_str()).collect();
    format!("{head}{depth}; tags {}", names.join(", "))
}

fn notice_text(notice: ExportNotice) -> String {
    match notice {
        ExportNotice::SeratoCuesShifted { cut_s } => format!(
            "the copy keeps its Serato cue points and beat grid as they are, so in Serato they sit \
             {:.2} s late; the original is unchanged",
            cut_s.0
        ),
    }
}

fn xml_only_text(reason: XmlOnlyReason) -> String {
    match reason {
        XmlOnlyReason::Mp3OrAac { codec } => {
            format!("{} (file writes arrive later)", codec.label())
        }
        XmlOnlyReason::GridOnlyFlac => {
            "grid only on FLAC (writing tags alone to FLAC arrives later)".to_owned()
        }
        XmlOnlyReason::GridOnlyWouldRequantise { float, bits } => {
            let bits = bits.map_or_else(String::new, |b| format!("{b}-bit "));
            let kind = if float { "float" } else { "integer" };
            format!("grid only would change the samples of this {bits}{kind} file")
        }
        XmlOnlyReason::NoTagToWriteGridOnly => {
            "grid only, and the file has no tag to write into".to_owned()
        }
    }
}

/// Why a file is skipped, and what to do.
fn skip_text(reason: ExportSkip) -> (String, String) {
    match reason {
        ExportSkip::SeratoInPlaceCut { cut_s } => (
            format!(
                "cutting {:.2} s in place would move its Serato cue points, which are stored as \
                 positions in the audio",
                cut_s.0
            ),
            "export it to a folder (the original keeps its cues), or use --batch-mode library"
                .to_owned(),
        ),
        ExportSkip::NotDjSafeRate { sample_rate_hz } => (
            format!(
                "its sample rate, {sample_rate_hz} Hz, is not one DJ players accept (44.1 or 48 kHz)"
            ),
            "convert it to 44.1 or 48 kHz in an audio editor first".to_owned(),
        ),
        ExportSkip::UnsupportedChannels { channels } => (
            format!(
                "it has {channels} channels; DJ players and SoundCheck write mono or stereo only"
            ),
            "export a stereo version first".to_owned(),
        ),
        ExportSkip::NothingToWrite { reason } => (
            format!(
                "only the rekordbox XML could carry its grid ({}), and the XML is off",
                xml_only_text(reason)
            ),
            "run it again without --no-xml".to_owned(),
        ),
        ExportSkip::Unsupported { codec } => (
            format!(
                "{} files are analysed only; SoundCheck does not write them",
                codec.label()
            ),
            "convert it to AIFF, WAV or FLAC first".to_owned(),
        ),
        ExportSkip::Silent => (
            "it has no audio above the -70 LUFS gate, so there is no loudness to align".to_owned(),
            "check that it is the right file".to_owned(),
        ),
        ExportSkip::NoGrid => (
            "grid only, and no beats were found, so there is no grid to write".to_owned(),
            "set the grid in the app, or export without --grid-only".to_owned(),
        ),
    }
}

#[cfg(test)]
mod tests;
