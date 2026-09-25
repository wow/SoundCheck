//! One file end to end: decode, measure loudness, read tag hints, cache. Beat tracking and the
//! grid solver join this pipeline as they land; the whole function moves into the engine crate
//! once batches exist, so nothing here is specific to the command line.

use std::io::Write;
use std::path::Path;

use sc_analysis::LoudnessMeter;
use sc_core::analysis::{AnalysisRecord, AnalysisSettings, RECORD_SCHEMA};
use sc_core::ipc::IpcError;
use sc_core::{Lu, Lufs, Result, SampleIndex};
use sc_io::cache::Cache;
use sc_io::{Decoder, tags};
use serde::Serialize;

/// Schema of the CLI's JSON output; bumped only with a breaking change.
pub const REPORT_SCHEMA: u32 = 2;

/// How the cache took part in a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CacheStatus {
    /// Served from the cache.
    Hit,
    /// Analysed and written to the cache.
    Written,
    /// Analysed with the cache switched off.
    Bypassed,
}

/// The CLI's JSON document for one analysed file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeReport {
    /// [`REPORT_SCHEMA`].
    pub schema: u32,
    /// Cache participation.
    pub cache: CacheStatus,
    /// The analysis.
    pub record: AnalysisRecord,
}

/// The CLI's JSON document for a file that failed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReport {
    /// [`REPORT_SCHEMA`].
    pub schema: u32,
    /// The file as given on the command line.
    pub file: String,
    /// Class and message.
    pub error: IpcError,
}

/// What one analysis run needs besides the file.
#[derive(Debug, Clone)]
pub struct Options {
    /// Analysis settings (part of the cache key).
    pub settings: AnalysisSettings,
    /// The cache to consult and fill; `None` bypasses it.
    pub cache: Option<Cache>,
}

/// Analyses `path`, serving from the cache when the entry is current.
///
/// # Errors
/// Decoding errors from [`Decoder`], [`sc_core::Error::Io`] when the file cannot be stat'ed or
/// the cache entry cannot be written, [`sc_core::Error::InvalidArgument`] from the meter.
pub fn analyze_file(path: &Path, opts: &Options) -> Result<AnalyzeReport> {
    let (nfc_path, key) = Cache::key_for(path, &opts.settings)?;
    if let Some(cache) = &opts.cache
        && let Some(record) = cache.get(&nfc_path, &key)
    {
        return Ok(AnalyzeReport {
            schema: REPORT_SCHEMA,
            cache: CacheStatus::Hit,
            record,
        });
    }

    let decoder = Decoder::open(path)?;
    let spec = decoder.spec();
    let (delay, padding) = (decoder.delay(), decoder.padding());
    let mut meter = LoudnessMeter::new(spec)?;
    let frames = decoder.for_each_block(|block| {
        meter.push(block);
        Ok(())
    })?;
    let loudness = meter.finish();
    let grid_skipped = if opts.settings.grid {
        "beat tracking is not available in this build"
    } else {
        "not requested"
    };
    let record = AnalysisRecord {
        schema: RECORD_SCHEMA,
        version: sc_core::VERSION.into(),
        path: nfc_path.clone(),
        size: key.size,
        mtime_ns: key.mtime_ns,
        spec,
        frames,
        duration: SampleIndex(frames).to_seconds(spec.sample_rate),
        delay,
        padding,
        loudness,
        grid: None,
        grid_skipped: Some(grid_skipped.into()),
        tags: tags::read_hints(path),
        evidence: None,
    };
    let cache = match &opts.cache {
        Some(cache) => {
            cache.put(&nfc_path, &key, &record)?;
            CacheStatus::Written
        }
        None => CacheStatus::Bypassed,
    };
    Ok(AnalyzeReport {
        schema: REPORT_SCHEMA,
        cache,
        record,
    })
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
        (Some(g), _) => writeln!(
            out,
            "  grid      {:.2} BPM ({})  bar 1 at {:.3} s  {:?}",
            g.bpm.0,
            g.meter,
            g.anchor.to_seconds(r.spec.sample_rate).0,
            g.verdict
        )?,
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

fn lufs(value: Option<Lufs>) -> String {
    value.map_or_else(|| "n/a".into(), |v| format!("{:.1} LUFS", v.0))
}

fn lu(value: Option<Lu>) -> String {
    value.map_or_else(|| "n/a".into(), |v| format!("{:.1} LU", v.0))
}
