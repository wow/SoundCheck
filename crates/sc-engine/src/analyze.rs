//! One file end to end: decode once, measuring loudness and building a 22 050 Hz mono copy in the
//! same pass; then beats, onsets, meter and grid; tag hints; cache.
//!
//! Cancellation is checked between decoded blocks, before and during beat tracking (once per
//! 30 s model chunk) and before the cache write, so a cancelled file never leaves a cache entry.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sc_analysis::beats::{
    BEAT_MODEL_FILE, BEAT_MODEL_FULL_FILE, BeatTracker, MODEL_SAMPLE_RATE, find_model_dir,
};
use sc_analysis::{LoudnessMeter, refit};
use sc_core::analysis::LoudnessReport;
use sc_core::analysis::{
    AnalysisRecord, AnalysisSettings, Grid, GridEdit, GridEvidence, Model, OnsetList,
    RECORD_SCHEMA, TagHints,
};
use sc_core::{AudioSpec, Error, Result, SampleIndex};
use sc_dsp::{KickBand, Onset, OnsetDetector, Resampler, to_mono};
use sc_io::cache::Cache;
use sc_io::{Decoder, tags};
use serde::Serialize;

use crate::cancel::CancelToken;

/// Schema of the per-file report (`sc-cli analyze --json`, the app's rows); bumped only with a
/// breaking change.
pub const REPORT_SCHEMA: u32 = 2;

/// Tracks shorter than this get loudness only.
const MIN_GRID_SECONDS: f64 = 10.0;

/// Shares of a file's analysis time reported as progress: decoding with loudness and
/// resampling, then beat tracking; onsets, meter and grid take the rest. Measured on an M1
/// (decode + loudness + resample about 18 %, beat tracking about 80 %).
const DECODE_SHARE: f32 = 0.2;
const BEATS_SHARE: f32 = 0.75;

/// Smallest progress step reported; finer steps are dropped at the source.
const PROGRESS_STEP: f32 = 0.01;

/// Receives a file's analysis progress, `0.0..=1.0`, from whichever thread runs it.
pub type Progress = Arc<dyn Fn(f32) + Send + Sync>;

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
    /// Checked between stages; the tracker is loaded with the same flag.
    pub cancel: CancelToken,
}

impl Analyzer {
    /// An analyzer for `settings`, loading the beat-tracking model (small or full, per
    /// `settings.model`) when the grid is on, tied to `cancel`.
    ///
    /// # Errors
    /// [`Error::ModelUnavailable`] naming the directories searched, [`Error::Internal`] when a
    /// model file cannot be parsed.
    pub fn load(
        settings: AnalysisSettings,
        cache: Option<Cache>,
        cancel: CancelToken,
    ) -> Result<Self> {
        let tracker = if settings.grid {
            let file = match settings.model {
                Model::Small => BEAT_MODEL_FILE,
                Model::Full => BEAT_MODEL_FULL_FILE,
            };
            Some(BeatTracker::load_with_cancel(
                &find_model_dir()?,
                file,
                cancel.flag(),
            )?)
        } else {
            None
        };
        Ok(Self {
            settings,
            cache,
            tracker,
            cancel,
        })
    }

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
        self.analyze_with(path, timings, None)
    }

    /// As [`Analyzer::analyze_timed`], reporting progress to `progress` in steps of at least 1 %.
    ///
    /// # Errors
    /// As for [`Analyzer::analyze`], and [`Error::Cancelled`] once `self.cancel` is set.
    pub fn analyze_with(
        &mut self,
        path: &Path,
        timings: &mut Timings,
        progress: Option<&Progress>,
    ) -> Result<AnalyzeReport> {
        self.run(path, timings, progress, true)
    }

    /// As [`Analyzer::analyze_with`], but never served from the cache: the file is analysed and
    /// its entry replaced. For a file just written, whose old entry may still match: a file
    /// rewritten at the same length with its modification time put back has the same key.
    ///
    /// # Errors
    /// As for [`Analyzer::analyze_with`].
    pub fn analyze_fresh(
        &mut self,
        path: &Path,
        timings: &mut Timings,
        progress: Option<&Progress>,
    ) -> Result<AnalyzeReport> {
        self.run(path, timings, progress, false)
    }

    fn run(
        &mut self,
        path: &Path,
        timings: &mut Timings,
        progress: Option<&Progress>,
        read_cache: bool,
    ) -> Result<AnalyzeReport> {
        self.check_cancel()?;
        let (nfc_path, key) = Cache::key_for(path, &self.settings)?;
        if read_cache
            && let Some(cache) = &self.cache
            && let Some(record) = cache.get(&nfc_path, &key)
        {
            return Ok(AnalyzeReport {
                schema: REPORT_SCHEMA,
                cache: CacheStatus::Hit,
                record,
            });
        }

        let want_grid = self.settings.grid && self.tracker.is_some();
        let decoded = decode_pass(path, want_grid, &self.cancel, progress, timings)?;
        let (spec, frames, mono_22k) = (decoded.spec, decoded.frames, decoded.mono_22k);
        let (delay, padding, loudness) = (decoded.delay, decoded.padding, decoded.loudness);

        self.check_cancel()?;
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
                progress,
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
        // A file cancelled during its last stage still leaves no cache entry.
        self.check_cancel()?;
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

impl Analyzer {
    fn check_cancel(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// What one decoding pass yields.
struct Decoded {
    spec: AudioSpec,
    frames: u64,
    delay: u32,
    padding: u32,
    loudness: LoudnessReport,
    mono_22k: Option<Vec<f32>>,
}

/// Decodes `path` once, feeding the loudness meter and (when the grid is wanted) a 22 050 Hz
/// mono copy, checking `cancel` between blocks and reporting the decode share of `progress`.
fn decode_pass(
    path: &Path,
    want_grid: bool,
    cancel: &CancelToken,
    progress: Option<&Progress>,
    timings: &mut Timings,
) -> Result<Decoded> {
    let start = Instant::now();
    let decoder = Decoder::open(path)?;
    let total_frames = decoder.total_frames().filter(|&t| t > 0);
    let spec = decoder.spec();
    let (delay, padding) = (decoder.delay(), decoder.padding());
    let mut loudness_meter = LoudnessMeter::new(spec)?;
    let mut resampler = if want_grid {
        Some(Resampler::new(spec.sample_rate, MODEL_SAMPLE_RATE)?)
    } else {
        None
    };
    let mut mono = Vec::new();
    let (mut in_loudness, mut in_resample) = (Duration::ZERO, Duration::ZERO);
    let share = if want_grid { DECODE_SHARE } else { 1.0 };
    let (mut frames_seen, mut reported) = (0_u64, 0.0_f32);
    let channels = u64::from(spec.channels);
    let frames = decoder.for_each_block(|block| {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        frames_seen += block.len() as u64 / channels;
        if let (Some(p), Some(total)) = (progress, total_frames) {
            // Display precision; frame counts fit f64 exactly.
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let fraction = share * (frames_seen as f64 / total as f64).min(1.0) as f32;
            if fraction >= reported + PROGRESS_STEP {
                reported = fraction;
                p(fraction);
            }
        }
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
    Ok(Decoded {
        spec,
        frames,
        delay,
        padding,
        loudness,
        mono_22k,
    })
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
    progress: Option<&Progress>,
) -> Result<GridOutcome> {
    let t = Instant::now();
    let raw = match progress {
        Some(p) => {
            let p = Arc::clone(p);
            tracker.track_with(mono, Box::new(move |f| p(DECODE_SHARE + BEATS_SHARE * f)))?
        }
        None => tracker.track(mono)?,
    };
    timings.beats += t.elapsed();

    let t = Instant::now();
    let detector = OnsetDetector::new(MODEL_SAMPLE_RATE);
    let mut band = mono.to_vec();
    KickBand::new(MODEL_SAMPLE_RATE).process_block(&mut band);
    let kick = detector.detect(&band);
    let broadband = detector.detect(mono);
    timings.onsets += t.elapsed();

    let t = Instant::now();
    let evidence = GridEvidence {
        beats_s: raw.beats_s.clone(),
        downbeats_s: raw.downbeats_s.clone(),
        downbeat_logits_50fps: raw.downbeat_logits.clone(),
        kick_onsets: onset_list(&kick),
        broadband_onsets: onset_list(&broadband),
    };
    let ctx = refit::Context {
        bpm_range: settings.bpm_range,
        tags: hints,
        sample_rate,
    };
    // The analysis is a refit with no edit, so the cached evidence reproduces this grid.
    let grid = refit::refit(&evidence, &ctx, &GridEdit::default()).map(|solved| solved.grid);
    timings.grid += t.elapsed();
    let skipped = grid.is_none().then(|| "no beats found".to_owned());
    Ok((grid, skipped, Some(evidence)))
}

/// Onsets as the cache stores them: frames at the analysis rate.
fn onset_list(onsets: &[Onset]) -> OnsetList {
    OnsetList {
        sample_rate: MODEL_SAMPLE_RATE,
        frames: onsets
            .iter()
            .map(|o| u32::try_from(o.frame).unwrap_or(u32::MAX))
            .collect(),
        rise_db: onsets.iter().map(|o| o.rise_db).collect(),
        level_db: onsets.iter().map(|o| o.level_db).collect(),
    }
}
