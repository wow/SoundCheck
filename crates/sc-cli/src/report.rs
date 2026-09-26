//! How `sc-cli` prints reports: the text form, and the JSON document for a file that failed.

use std::io::Write;

use sc_core::analysis::Grid;
use sc_core::ipc::IpcError;
use sc_core::{Lu, Lufs};
use sc_engine::{AnalyzeReport, CacheStatus};
use serde::Serialize;

/// The CLI's JSON document for a file that failed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReport {
    /// [`sc_engine::REPORT_SCHEMA`].
    pub schema: u32,
    /// The file as given on the command line.
    pub file: String,
    /// Class and message.
    pub error: IpcError,
}

/// The human-readable form of a report.
///
/// # Errors
/// Whatever `out` returns.
pub fn write_text(report: &AnalyzeReport, out: &mut impl Write) -> std::io::Result<()> {
    let r = &report.record;
    let l = &r.loudness;
    writeln!(out, "{}", r.path)?;
    write!(
        out,
        "  {} Hz, {} ch, {} frames ({:.3} s)",
        r.spec.sample_rate, r.spec.channels, r.frames, r.duration.0
    )?;
    if r.delay > 0 || r.padding > 0 {
        write!(
            out,
            ", encoder delay {} + padding {} removed",
            r.delay, r.padding
        )?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "  loudness  I {}  S-P95 {}  max-S {}  max-M {}  LRA {}  TP {:.2} dBTP  PLR {}{}",
        lufs(l.integrated),
        lufs(l.short_term_p95),
        lufs(l.short_term_max),
        lufs(l.momentary_max),
        lu(l.lra),
        l.true_peak.0,
        lu(l.plr),
        if l.dual_mono {
            "  (mono, measured as dual mono)"
        } else {
            ""
        }
    )?;
    match (&r.grid, &r.grid_skipped) {
        (Some(g), _) => write_grid(g, r.spec.sample_rate, out)?,
        (None, Some(why)) => writeln!(out, "  grid      {why}")?,
        (None, None) => {}
    }
    if let Some(bpm) = r.tags.bpm {
        writeln!(out, "  tags      BPM {:.2}", bpm.0)?;
    }
    let cache = match report.cache {
        CacheStatus::Hit => "hit",
        CacheStatus::Written => "written",
        CacheStatus::Bypassed => "bypassed",
    };
    writeln!(out, "  cache     {cache}")
}

fn write_grid(g: &Grid, sample_rate: u32, out: &mut impl Write) -> std::io::Result<()> {
    let verdict = match g.verdict {
        sc_core::Verdict::Static => "static",
        sc_core::Verdict::StaticWarn => "static (check)",
        sc_core::Verdict::Drifts => "drifts",
    };
    writeln!(
        out,
        "  grid      {:.2} BPM ({})  bar 1 at {:.3} s  {verdict}  residual p95 {:.0} ms / max {:.0} ms  drift {:+.0} ppm",
        g.bpm.0,
        g.meter,
        g.anchor.to_seconds(sample_rate).0,
        g.residual_p95_ms,
        g.residual_max_ms,
        g.drift_ppm
    )?;
    let confidence = match g.confidence {
        sc_core::Confidence::Green => "green",
        sc_core::Confidence::Amber => "amber",
        sc_core::Confidence::Red => "red",
    };
    let mut alternatives = Vec::new();
    if let Some(up) = g.alternatives.octave_up {
        alternatives.push(format!("x2 {:.2}", up.0));
    }
    if let Some(down) = g.alternatives.octave_down {
        alternatives.push(format!("/2 {:.2}", down.0));
    }
    if !g.alternatives.downbeat_shift_beats.is_empty() {
        let shifts: Vec<String> = g
            .alternatives
            .downbeat_shift_beats
            .iter()
            .map(|s| format!("{s:+}"))
            .collect();
        alternatives.push(format!("bar 1 {}", shifts.join("/")));
    }
    if let Some(m) = &g.meter_runner_up {
        alternatives.push(format!("meter {m}"));
    }
    write!(out, "  confidence {confidence}")?;
    if !g.reasons.is_empty() {
        let reasons: Vec<String> = g.reasons.iter().map(|r| format!("{r:?}")).collect();
        write!(out, " ({})", reasons.join(", "))?;
    }
    writeln!(out, "   alternatives {}", alternatives.join(", "))
}

fn lufs(value: Option<Lufs>) -> String {
    value.map_or_else(|| "n/a".into(), |v| format!("{:.1} LUFS", v.0))
}

fn lu(value: Option<Lu>) -> String {
    value.map_or_else(|| "n/a".into(), |v| format!("{:.1} LU", v.0))
}
