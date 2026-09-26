//! One file end to end: decode once, measuring loudness and building a 22 050 Hz mono copy in the
//! same pass; then beats, onsets, meter and grid; tag hints; cache.

use std::path::Path;
use std::time::{Duration, Instant};

use sc_analysis::beats::{BeatTracker, MODEL_SAMPLE_RATE};
use sc_analysis::grid::{self, Evidence, SolveSettings, TimedOnset};
use sc_analysis::{LoudnessMeter, meter};
use sc_core::analysis::{
    AnalysisRecord, AnalysisSettings, BeatUnit, Grid, GridEvidence, RECORD_SCHEMA, TagHints,
};
use sc_core::{Result, SampleIndex, Seconds};
use sc_dsp::{KickBand, Onset, OnsetDetector, Resampler, to_mono};
use sc_io::cache::Cache;
use sc_io::{Decoder, tags};
use serde::Serialize;

/// Schema of the per-file report (`sc-cli analyze --json`, the app's rows); bumped only with a
/// breaking change.
pub const REPORT_SCHEMA: u32 = 2;

/// Tracks shorter than this get loudness only.
const MIN_GRID_SECONDS: f64 = 10.0;

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

/// The report for one analysed file: the record and how the cache took part.
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

/// Wall time per stage of one analysis.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    /// Decoding (the file's blocks, excluding what the consumers do with them).
    pub decode: Duration,
    /// Loudness metering.
    pub loudness: Duration,
    /// Mono fold-down and resampling to 22 050 Hz.
    pub resample: Duration,
    /// Beat tracking.
    pub beats: Duration,
    /// Kick-band and broadband onset detection.
    pub onsets: Duration,
    /// Meter estimation and grid solving.
    pub grid: Duration,
}

/// The analysis pipeline with its settings, cache and (when the grid is on) a loaded model.
#[derive(Debug)]
pub struct Analyzer {
    /// Analysis settings (part of the cache key).
    pub settings: AnalysisSettings,
    /// The cache to consult and fill; `None` bypasses it.
    pub cache: Option<Cache>,
    /// The beat tracker; required when `settings.grid` is on.
    pub tracker: Option<BeatTracker>,
}

impl Analyzer {
    /// Analyses `path`, serving from the cache when the entry is current.
    ///
    /// # Errors
    /// Decoding errors from [`Decoder`], [`sc_core::Error::Io`] when the file cannot be stat'ed
    /// or the cache entry cannot be written, [`sc_core::Error::Internal`] when beat tracking
    /// fails.
    pub fn analyze(&mut self, path: &Path) -> Result<AnalyzeReport> {
        self.analyze_timed(path, &mut Timings::default())
    }

    /// As [`Analyzer::analyze`], accumulating the time of each stage into `timings`.
    ///
    /// # Errors
    /// As for [`Analyzer::analyze`].
    pub fn analyze_timed(&mut self, path: &Path, timings: &mut Timings) -> Result<AnalyzeReport> {
        let (nfc_path, key) = Cache::key_for(path, &self.settings)?;
        if let Some(cache) = &self.cache
            && let Some(record) = cache.get(&nfc_path, &key)
        {
            return Ok(AnalyzeReport {
                schema: REPORT_SCHEMA,
                cache: CacheStatus::Hit,
                record,
            });
        }

        let start = Instant::now();
        let decoder = Decoder::open(path)?;
        let spec = decoder.spec();
        let (delay, padding) = (decoder.delay(), decoder.padding());
        let mut loudness_meter = LoudnessMeter::new(spec)?;
        let want_grid = self.settings.grid && self.tracker.is_some();
        let mut resampler = if want_grid {
            Some(Resampler::new(spec.sample_rate, MODEL_SAMPLE_RATE)?)
        } else {
            None
        };
        let mut mono = Vec::new();
        let (mut in_loudness, mut in_resample) = (Duration::ZERO, Duration::ZERO);
        let frames = decoder.for_each_block(|block| {
            let t = Instant::now();
            loudness_meter.push(block);
            in_loudness += t.elapsed();
            if let Some(r) = resampler.as_mut() {
                let t = Instant::now();
                mono.clear();
                to_mono(block, spec.channels, &mut mono);
                r.push(&mono);
                in_resample += t.elapsed();
            }
            Ok(())
        })?;
        let t = Instant::now();
        let loudness = loudness_meter.finish();
        in_loudness += t.elapsed();
        let t = Instant::now();
        let mono_22k = resampler.map(Resampler::finish);
        in_resample += t.elapsed();
        timings.decode += start.elapsed().saturating_sub(in_loudness + in_resample);
        timings.loudness += in_loudness;
        timings.resample += in_resample;

        let duration = SampleIndex(frames).to_seconds(spec.sample_rate);
        let hints = tags::read_hints(path);
        let (grid, grid_skipped, evidence) = match (&mono_22k, self.tracker.as_mut()) {
            _ if !self.settings.grid => (None, Some("not requested".to_owned()), None),
            (Some(mono), Some(tracker)) if duration.0 >= MIN_GRID_SECONDS => grid_for(
                mono,
                spec.sample_rate,
                &self.settings,
                &hints,
                tracker,
                timings,
            )?,
            (Some(_), Some(_)) => (None, Some("shorter than 10 s".to_owned()), None),
            _ => (
                None,
                Some("beat-tracking model not loaded".to_owned()),
                None,
            ),
        };

        let record = AnalysisRecord {
            schema: RECORD_SCHEMA,
            version: sc_core::VERSION.into(),
            path: nfc_path.clone(),
            size: key.size,
            mtime_ns: key.mtime_ns,
            spec,
            frames,
            duration,
            delay,
            padding,
            loudness,
            grid,
            grid_skipped,
            tags: hints,
            evidence,
        };
        let cache = match &self.cache {
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
}

type GridOutcome = (Option<Grid>, Option<String>, Option<GridEvidence>);

/// Beats, onsets, meter and grid for a 22 050 Hz mono signal.
fn grid_for(
    mono: &[f32],
    sample_rate: u32,
    settings: &AnalysisSettings,
    hints: &TagHints,
    tracker: &mut BeatTracker,
    timings: &mut Timings,
) -> Result<GridOutcome> {
    let t = Instant::now();
    let raw = tracker.track(mono)?;
    timings.beats += t.elapsed();

    let t = Instant::now();
    let detector = OnsetDetector::new(MODEL_SAMPLE_RATE);
    let mut band = mono.to_vec();
    KickBand::new(MODEL_SAMPLE_RATE).process_block(&mut band);
    let kick = timed(&detector.detect(&band));
    let broadband = timed(&detector.detect(mono));
    timings.onsets += t.elapsed();

    let t = Instant::now();
    let beats_s: Vec<f64> = raw.beats_s.iter().map(|&b| f64::from(b)).collect();
    let downbeats_s: Vec<f64> = raw.downbeats_s.iter().map(|&b| f64::from(b)).collect();
    let evidence = GridEvidence {
        beats: raw.beats_at(sample_rate),
        downbeats: raw.downbeats_at(sample_rate),
        downbeat_logits_50fps: raw.downbeat_logits.clone(),
        kick_onsets: kick
            .iter()
            .map(|o| Seconds(o.time_s).to_sample_index(sample_rate))
            .collect(),
    };
    let Some(fit) = grid::fit_beats(&beats_s) else {
        timings.grid += t.elapsed();
        return Ok((None, Some("no beats found".to_owned()), Some(evidence)));
    };
    let hint = [&hints.genre, &hints.title, &hints.artist]
        .iter()
        .filter_map(|s| s.as_deref())
        .collect::<Vec<_>>()
        .join(" ");
    // The meter is read on the attacks' phase, not the model's: trackers sometimes follow the
    // off-beat for whole sections.
    let anchor_onsets = if kick.len() * 4 >= beats_s.len() {
        &kick
    } else {
        &broadband
    };
    let phase = grid::onset_phase(&fit, anchor_onsets);
    let estimate = meter::estimate(
        fit.period,
        phase,
        fit.span,
        &kick,
        &broadband,
        &raw.downbeat_logits,
        &hint,
    );
    let solve_settings = SolveSettings {
        bpm_range: (settings.bpm_range.0.0, settings.bpm_range.1.0),
        tag_bpm: hints.bpm.map(|b| b.0),
        genre: hints.genre.clone(),
        meter: estimate.meter.clone(),
        meter_margin: estimate.margin,
        fixed_period: (estimate.meter.unit == BeatUnit::Eighth).then_some(estimate.unit_period),
        ..SolveSettings::default()
    };
    let ev = Evidence {
        beats_s: &beats_s,
        downbeats_s: &downbeats_s,
        downbeat_logits: &raw.downbeat_logits,
        kick_onsets: &kick,
        broadband_onsets: &broadband,
    };
    let grid = grid::solve(&ev, &solve_settings, sample_rate).map(|mut g| {
        g.meter_runner_up.clone_from(&estimate.runner_up);
        g
    });
    timings.grid += t.elapsed();
    let skipped = grid.is_none().then(|| "no beats found".to_owned());
    Ok((grid, skipped, Some(evidence)))
}

fn timed(onsets: &[Onset]) -> Vec<TimedOnset> {
    onsets
        .iter()
        .map(|o| TimedOnset {
            time_s: SampleIndex(o.frame as u64).to_seconds(MODEL_SAMPLE_RATE).0,
            rise_db: o.rise_db,
            level_db: o.level_db,
        })
        .collect()
}
