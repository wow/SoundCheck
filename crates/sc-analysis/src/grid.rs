//! The static beat grid: tempo, phase, octave, bar 1, residuals, verdict and confidence.
//!
//! 1. **Tempo** from the model's beats by robust least squares: Huber weights (20 ms) with hard
//!    rejection beyond 40 ms, over beat indices assigned progressively (the first 16 beats by
//!    their spacing, then windows doubling in length, each indexed by the previous fit), so
//!    missing and spurious beats neither bias the tempo nor shift the count. No beat is snapped
//!    to a transient: at the model's 20 ms resolution that adds structured noise, while the
//!    global fit averages it out (768 beats give about 0.0005 BPM).
//! 2. **Phase**: one global offset from correlating the onset train with a comb at the fitted
//!    period, within +/-40 ms of the beats' own phase (the model's beats can sit a frame or two
//!    off the attacks), then centred on the median offset of the attacks near the comb peak.
//! 3. **Octave**: inside the user's DJ-app BPM range first, then the file's BPM tag (2 %), the
//!    genre's usual range, the kick density on each lattice, and finally the model's own choice.
//! 4. **Round BPM** only when the round value lies within max(3 sigma, 0.002) BPM of the fit: a
//!    true 127.98 exported as 128.00 drifts 56 ms over six minutes.
//! 5. **Bar 1**: the meter's downbeat phase from the model's downbeat activations plus the onset
//!    accents gives the lattice point; the anchor moves onto the earliest significant rise
//!    (within 6 dB of the strongest within +/-40 ms) only when that rise agrees with the global
//!    phase to 5 ms, so a single early or late kick never shifts the whole grid.
//! 6. **Fitness**: onset residuals against the final grid (P95 and maximum), local tempo over
//!    128-beat windows (hop 32) and a quadratic drift give the verdict; coverage, recall and the
//!    octave, downbeat and meter margins give a calibrated three-state confidence.

use sc_core::analysis::{Alternatives, Confidence, Grid, Meter, Reason, Verdict};
use sc_core::{Bpm, SampleIndex, Seconds};

/// Fewest beats (after removing double detections) a grid is fitted to.
pub const MIN_BEATS: usize = 8;
/// Beats below this count make the grid at most amber (reason `Short`).
const SHORT_BEATS: usize = 32;
/// Huber threshold of the tempo fit.
const HUBER_S: f64 = 0.020;
/// Residual beyond which a beat is ignored by the tempo fit.
const REJECT_S: f64 = 0.040;
/// Search range, step and kernel half-width of the comb phase.
const COMB_RANGE_S: f64 = 0.040;
const COMB_STEPS: i32 = 80;
const COMB_KERNEL_S: f64 = 0.006;
/// Window around the bar-1 lattice point in which the anchor onset is sought.
const ANCHOR_WINDOW_S: f64 = 0.040;
/// A rise within this many dB of the strongest one in the window counts as significant.
const ANCHOR_SIGNIFICANCE_DB: f32 = 6.0;
/// The anchor moves to that rise only when it agrees with the global phase this closely;
/// otherwise one early or late kick would shift the whole grid.
const ANCHOR_AGREE_S: f64 = 0.005;
/// Largest distance at which an onset is matched to a grid line (never more than a quarter
/// beat).
const MATCH_WINDOW_S: f64 = 0.050;
/// Distance within which a model beat counts as on the lattice (coverage, recall, density).
const INLIER_S: f64 = 0.025;
/// Frames per second of the model's activations.
const MODEL_FPS: f64 = 50.0;
/// Verdict thresholds.
const STATIC_P95_MS: f64 = 12.0;
const STATIC_MAX_MS: f64 = 30.0;
const WARN_P95_MS: f64 = 25.0;
const WARN_MAX_MS: f64 = 50.0;
const LOCAL_RANGE_BPM: f64 = 0.03;
const DRIFT_PPM: f64 = 200.0;
const LOCAL_WINDOW: f64 = 128.0;
const LOCAL_HOP: f64 = 32.0;
/// Confidence cut points.
const GREEN: f64 = 0.8;
const AMBER: f64 = 0.5;
/// Accent difference (dB) worth as much as the full range of the downbeat activation.
const ACCENT_SCALE_DB: f64 = 6.0;

/// An onset on the file's timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedOnset {
    /// Time in seconds from the start of the decoded audio.
    pub time_s: f64,
    /// Energy rise in dB (how sharp the attack is).
    pub rise_db: f32,
    /// Envelope level just after the rise, in dBFS (how loud the attack is).
    pub level_db: f32,
}

/// What the solver works from, all on the decoded file's timeline.
#[derive(Debug, Clone, Copy)]
pub struct Evidence<'a> {
    /// Model beat times in seconds.
    pub beats_s: &'a [f64],
    /// Model downbeat activation (raw logits) per 20 ms frame.
    pub downbeat_logits: &'a [f32],
    /// Kick-band (30-150 Hz) onsets.
    pub kick_onsets: &'a [TimedOnset],
    /// Broadband onsets, the anchor's fallback when the kick band is silent.
    pub broadband_onsets: &'a [TimedOnset],
}

/// Everything besides the audio that shapes the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct SolveSettings {
    /// The user's DJ-app BPM range.
    pub bpm_range: (f64, f64),
    /// BPM from the file's tags.
    pub tag_bpm: Option<f64>,
    /// Genre from the file's tags.
    pub genre: Option<String>,
    /// The bar's pulse count and grouping.
    pub meter: Meter,
    /// The meter estimator's margin (1.0 when the user chose the meter).
    pub meter_margin: f64,
    /// The period of one grid beat when the meter fixes it (the eighth-note pulse of the x/8
    /// meters); skips the octave choice.
    pub fixed_period: Option<f64>,
    /// A tempo set by the user; skips the octave choice.
    pub bpm_override: Option<f64>,
    /// A bar-1 position set by the user, in seconds; used exactly.
    pub anchor_override_s: Option<f64>,
}

impl Default for SolveSettings {
    fn default() -> Self {
        Self {
            bpm_range: (70.0, 180.0),
            tag_bpm: None,
            genre: None,
            meter: Meter::four_four(),
            meter_margin: 1.0,
            fixed_period: None,
            bpm_override: None,
            anchor_override_s: None,
        }
    }
}

/// Fits the grid; `None` when fewer than [`MIN_BEATS`] usable beats exist.
#[must_use]
pub fn solve(ev: &Evidence<'_>, settings: &SolveSettings, sample_rate: u32) -> Option<Grid> {
    let beats = clean_beats(ev.beats_s);
    let tempo = fit_tempo(&beats)?;
    let span = (beats[0], beats[beats.len() - 1]);

    // Kick band unless it is (nearly) silent; then broadband attacks.
    let kick_enough = ev.kick_onsets.len() * 4 >= beats.len();
    let (onsets, used_broadband) = if kick_enough || ev.broadband_onsets.is_empty() {
        (ev.kick_onsets, false)
    } else {
        (ev.broadband_onsets, true)
    };

    // Coverage and recall on the model's own lattice.
    let (coverage, recall) = coverage_recall(&beats, tempo.period, tempo.phase);

    // Global phase from the onsets.
    let coarse = tempo.phase + comb_offset(onsets, tempo.period, tempo.phase, span);
    let phase = coarse + median_offset(onsets, tempo.period, coarse, span);

    // Octave (or the user's tempo).
    let octave = match settings.bpm_override {
        Some(bpm) if bpm > 0.0 => Octave {
            period: 60.0 / bpm,
            phase,
            rule: OctaveRule::Override,
            margin: 1.0,
        },
        _ => match settings.fixed_period {
            Some(period) if period > 0.0 => Octave {
                period,
                phase,
                rule: OctaveRule::Range,
                margin: 1.0,
            },
            _ => choose_octave(&tempo, phase, settings, onsets, span),
        },
    };
    let sigma_period = tempo.sigma_period * octave.period / tempo.period;
    let fitted_bpm = 60.0 / octave.period;
    let bpm = if octave.rule == OctaveRule::Override {
        fitted_bpm
    } else {
        snap_bpm(
            fitted_bpm,
            60.0 * sigma_period / (octave.period * octave.period),
        )
    };
    let period = 60.0 / bpm;

    // Bar 1.
    let bar = usize::from(settings.meter.beats_per_bar.max(1));
    let (anchor_s, downbeat) = if let Some(anchor) = settings.anchor_override_s {
        (anchor.max(0.0), Downbeat::pinned())
    } else {
        let db = downbeat_phase(period, octave.phase, bar, span, ev.downbeat_logits, onsets);
        let lattice = first_downbeat(period, octave.phase, bar, db.r, span.0);
        let local = snap_anchor(lattice, onsets).filter(|t| (t - lattice).abs() <= ANCHOR_AGREE_S);
        (local.unwrap_or(lattice), db)
    };

    let fit = fitness(anchor_s, period, onsets, &beats, span);
    let verdict = verdict(&fit);
    let first_downbeat_index = nearest_index(ev.beats_s, anchor_s);

    let (confidence, reasons) = assess(&Assessment {
        coverage,
        recall,
        octave_margin: octave.margin,
        downbeat_margin: downbeat.margin,
        verdict,
        bpm,
        used_broadband,
        beats: beats.len(),
        settings,
    });

    Some(Grid {
        anchor: Seconds(anchor_s).to_sample_index(sample_rate),
        bpm: Bpm(bpm),
        meter: settings.meter.clone(),
        meter_runner_up: None,
        first_downbeat_index,
        phrase_len_bars: 8,
        segments: Vec::new(),
        residual_p95_ms: to_f32(fit.p95_ms),
        residual_max_ms: to_f32(fit.max_ms),
        local_bpm_range: to_f32(fit.local_range_bpm),
        drift_ppm: to_f32(fit.drift_ppm),
        verdict,
        confidence,
        reasons,
        alternatives: Alternatives {
            octave_up: (bpm * 2.0 <= 400.0).then_some(Bpm(bpm * 2.0)),
            octave_down: (bpm / 2.0 >= 30.0).then_some(Bpm(bpm / 2.0)),
            downbeat_shift_beats: downbeat.shifts,
        },
    })
}

/// The inputs of the confidence and the reason chips.
struct Assessment<'a> {
    coverage: f64,
    recall: f64,
    octave_margin: f64,
    downbeat_margin: f64,
    verdict: Verdict,
    bpm: f64,
    used_broadband: bool,
    beats: usize,
    settings: &'a SolveSettings,
}

/// Calibrated three-state confidence from the weakest term, and the chips explaining it.
fn assess(a: &Assessment<'_>) -> (Confidence, Vec<Reason>) {
    let mut reasons = Vec::new();
    let terms = [
        (a.coverage, Reason::Coverage),
        (a.recall, Reason::Recall),
        (a.octave_margin, Reason::OctaveMargin),
        (a.downbeat_margin, Reason::DownbeatMargin),
        (a.settings.meter_margin, Reason::MeterMargin),
    ];
    let mut weakest = terms.iter().map(|t| t.0).fold(1.0_f64, f64::min);
    for (value, reason) in terms {
        if value < GREEN {
            reasons.push(reason);
        }
    }
    match a.verdict {
        Verdict::Static => {}
        Verdict::StaticWarn => reasons.push(Reason::Residuals),
        Verdict::Drifts => reasons.push(Reason::Drifts),
    }
    let (lo, hi) = a.settings.bpm_range;
    if a.bpm < lo - 1e-9 || a.bpm > hi + 1e-9 {
        reasons.push(Reason::OutsideRange);
    }
    if let Some(tag) = a.settings.tag_bpm
        && tag > 0.0
        && (a.bpm - tag).abs() / tag > 0.02
    {
        reasons.push(Reason::TagDisagrees);
    }
    if a.used_broadband {
        reasons.push(Reason::NoKick);
    }
    if a.beats < SHORT_BEATS {
        reasons.push(Reason::Short);
        weakest = weakest.min(AMBER);
    }
    let confidence = if weakest >= GREEN {
        Confidence::Green
    } else if weakest >= AMBER {
        Confidence::Amber
    } else {
        Confidence::Red
    };
    (confidence, reasons)
}

/// The tempo fit of the model's beats alone: beat period and phase (seconds) and the span of
/// the usable beats. The meter estimator works on this lattice before the grid is solved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatFit {
    /// Seconds per model beat.
    pub period: f64,
    /// Time of lattice index 0, in seconds.
    pub phase: f64,
    /// First and last usable beat, in seconds.
    pub span: (f64, f64),
}

/// Fits the model's beats; `None` when fewer than [`MIN_BEATS`] usable beats exist.
#[must_use]
pub fn fit_beats(beats_s: &[f64]) -> Option<BeatFit> {
    let beats = clean_beats(beats_s);
    let tempo = fit_tempo(&beats)?;
    Some(BeatFit {
        period: tempo.period,
        phase: tempo.phase,
        span: (beats[0], beats[beats.len() - 1]),
    })
}

/// Samples of a grid line: `anchor + i * 60 * sample_rate / bpm`, rounded.
#[must_use]
pub fn line_at(grid: &Grid, i: i64, sample_rate: u32) -> SampleIndex {
    let t = grid.anchor.to_seconds(sample_rate).0 + index_f64(i) * 60.0 / grid.bpm.0;
    Seconds(t).to_sample_index(sample_rate)
}

// ---------------------------------------------------------------------------------------------
// Tempo

#[derive(Debug, Clone, Copy)]
struct Tempo {
    period: f64,
    phase: f64,
    sigma_period: f64,
}

#[derive(Debug, Clone, Copy)]
struct Line {
    intercept: f64,
    slope: f64,
}

/// Sorted, finite, non-negative beats without double detections (closer than 30 % of the
/// median spacing to the previous kept beat).
fn clean_beats(beats: &[f64]) -> Vec<f64> {
    let mut sorted: Vec<f64> = beats
        .iter()
        .copied()
        .filter(|t| t.is_finite() && *t >= 0.0)
        .collect();
    sorted.sort_by(f64::total_cmp);
    let mut increments: Vec<f64> = sorted
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.0)
        .collect();
    if increments.is_empty() {
        return sorted;
    }
    let spacing = median(&mut increments);
    let mut kept: Vec<f64> = Vec::with_capacity(sorted.len());
    for t in sorted {
        if kept.last().is_none_or(|&prev| t - prev >= 0.3 * spacing) {
            kept.push(t);
        }
    }
    kept
}

fn fit_tempo(beats: &[f64]) -> Option<Tempo> {
    if beats.len() < MIN_BEATS {
        return None;
    }
    let mut increments: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    let spacing = median(&mut increments);
    if spacing <= 0.0 {
        return None;
    }
    let mut n = beats.len().min(16);
    let mut line = irls(&incremental_pairs(&beats[..n], spacing))?;
    while n < beats.len() {
        n = (n * 2).min(beats.len());
        line = irls(&global_pairs(&beats[..n], line))?;
    }
    let mut pairs = global_pairs(beats, line);
    for _ in 0..2 {
        line = irls(&pairs)?;
        pairs = global_pairs(beats, line);
    }
    if line.slope <= 0.0 {
        return None;
    }
    let sigma_period = slope_sigma(&pairs, line);
    Some(Tempo {
        period: line.slope,
        phase: line.intercept,
        sigma_period,
    })
}

/// Indices from the spacing to the last kept beat; a beat less than half a spacing after it is
/// a spurious detection and is skipped.
fn incremental_pairs(beats: &[f64], spacing: f64) -> Vec<(f64, f64)> {
    let mut k = 0.0;
    let mut pairs = vec![(0.0, beats[0])];
    let mut last = beats[0];
    for &t in &beats[1..] {
        let steps = ((t - last) / spacing).round();
        if steps < 1.0 {
            continue;
        }
        k += steps;
        pairs.push((k, t));
        last = t;
    }
    pairs
}

/// Indices from a fitted line; of two beats on one index the closer one is kept.
fn global_pairs(beats: &[f64], line: Line) -> Vec<(f64, f64)> {
    let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(beats.len());
    for &t in beats {
        let k = ((t - line.intercept) / line.slope).round();
        let r = (t - (line.intercept + line.slope * k)).abs();
        if let Some(last) = pairs.last_mut()
            && (last.0 - k).abs() < 0.5
        {
            let r_last = (last.1 - (line.intercept + line.slope * k)).abs();
            if r < r_last {
                *last = (k, t);
            }
            continue;
        }
        pairs.push((k, t));
    }
    pairs
}

/// Iteratively reweighted least squares of `t = intercept + slope * k`.
fn irls(pairs: &[(f64, f64)]) -> Option<Line> {
    let mut weights = vec![1.0; pairs.len()];
    let mut line = weighted_line(pairs, &weights)?;
    for iteration in 0..8 {
        for (w, &(k, t)) in weights.iter_mut().zip(pairs) {
            let r = (t - (line.intercept + line.slope * k)).abs();
            *w = if iteration >= 2 && r > REJECT_S {
                0.0
            } else if r <= HUBER_S {
                1.0
            } else {
                HUBER_S / r
            };
        }
        line = weighted_line(pairs, &weights)?;
    }
    Some(line)
}

fn weighted_line(pairs: &[(f64, f64)], weights: &[f64]) -> Option<Line> {
    let sw: f64 = weights.iter().sum();
    if sw <= 0.0 {
        return None;
    }
    let mk = pairs.iter().zip(weights).map(|(p, w)| p.0 * w).sum::<f64>() / sw;
    let mt = pairs.iter().zip(weights).map(|(p, w)| p.1 * w).sum::<f64>() / sw;
    let (mut skk, mut skt) = (0.0, 0.0);
    for (&(k, t), &w) in pairs.iter().zip(weights) {
        skk += w * (k - mk) * (k - mk);
        skt += w * (k - mk) * (t - mt);
    }
    if skk <= 0.0 {
        return None;
    }
    let slope = skt / skk;
    Some(Line {
        intercept: mt - slope * mk,
        slope,
    })
}

/// Standard error of the slope over the inliers.
fn slope_sigma(pairs: &[(f64, f64)], line: Line) -> f64 {
    let inliers: Vec<(f64, f64)> = pairs
        .iter()
        .copied()
        .filter(|&(k, t)| (t - (line.intercept + line.slope * k)).abs() <= REJECT_S)
        .collect();
    if inliers.len() < 3 {
        return f64::INFINITY;
    }
    let n = count_f64(inliers.len());
    let mk = inliers.iter().map(|p| p.0).sum::<f64>() / n;
    let skk: f64 = inliers.iter().map(|p| (p.0 - mk) * (p.0 - mk)).sum();
    let ss: f64 = inliers
        .iter()
        .map(|&(k, t)| (t - (line.intercept + line.slope * k)).powi(2))
        .sum();
    if skk <= 0.0 {
        return f64::INFINITY;
    }
    (ss / (n - 2.0)).sqrt() / skk.sqrt()
}

fn coverage_recall(beats: &[f64], period: f64, phase: f64) -> (f64, f64) {
    let on_lattice = |t: f64| lattice_distance(t, period, phase).abs() <= INLIER_S;
    let covered = beats.iter().filter(|&&t| on_lattice(t)).count();
    let coverage = count_f64(covered) / count_f64(beats.len());
    let span = (beats[0], beats[beats.len() - 1]);
    let recall = slot_hits(beats, period, phase, span, INLIER_S);
    (coverage, recall)
}

// ---------------------------------------------------------------------------------------------
// Phase and octave

/// Signed distance from `t` to the nearest lattice line.
fn lattice_distance(t: f64, period: f64, phase: f64) -> f64 {
    let x = (t - phase) / period;
    (x - x.round()) * period
}

/// Offset (seconds) that best aligns the onsets with the lattice, within +/-40 ms.
fn comb_offset(onsets: &[TimedOnset], period: f64, phase: f64, span: (f64, f64)) -> f64 {
    let relevant: Vec<&TimedOnset> = onsets
        .iter()
        .filter(|o| o.time_s >= span.0 - period && o.time_s <= span.1 + period)
        .collect();
    if relevant.is_empty() {
        return 0.0;
    }
    let score = |offset: f64| -> f64 {
        relevant
            .iter()
            .map(|o| {
                let d = lattice_distance(o.time_s, period, phase + offset).abs();
                f64::from(o.rise_db.max(0.0)) * (1.0 - d / COMB_KERNEL_S).max(0.0)
            })
            .sum()
    };
    let step = COMB_RANGE_S / f64::from(COMB_STEPS);
    let mut best = (0.0, score(0.0));
    for i in 1..=COMB_STEPS {
        for sign in [1.0, -1.0] {
            let offset = sign * f64::from(i) * step;
            let s = score(offset);
            if s > best.1 + 1e-9 {
                best = (offset, s);
            }
        }
    }
    best.0
}

/// Median signed distance of the onsets within [`INLIER_S`] of the lattice: the comb finds the
/// peak region, this centres it (a plateau of evenly spread attacks has no single comb maximum).
fn median_offset(onsets: &[TimedOnset], period: f64, phase: f64, span: (f64, f64)) -> f64 {
    let window = INLIER_S.min(period / 4.0);
    let mut offsets: Vec<f64> = onsets
        .iter()
        .filter(|o| o.time_s >= span.0 - window && o.time_s <= span.1 + window)
        .map(|o| lattice_distance(o.time_s, period, phase))
        .filter(|d| d.abs() <= window)
        .collect();
    if offsets.is_empty() {
        0.0
    } else {
        median(&mut offsets)
    }
}

/// Fraction of lattice slots within `span` that hold an event within `window`.
fn slot_hits(events: &[f64], period: f64, phase: f64, span: (f64, f64), window: f64) -> f64 {
    let first = ((span.0 - phase) / period).ceil();
    let last = ((span.1 - phase) / period).floor();
    if last < first {
        return 0.0;
    }
    let slots = last - first + 1.0;
    let window = window.min(period / 4.0);
    let mut hit: Vec<f64> = events
        .iter()
        .filter_map(|&t| {
            let x = (t - phase) / period;
            let k = x.round();
            ((x - k).abs() * period <= window && k >= first && k <= last).then_some(k)
        })
        .collect();
    hit.sort_by(f64::total_cmp);
    hit.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    count_f64(hit.len()) / slots
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OctaveRule {
    Range,
    Tag,
    Genre,
    Density,
    ModelOctave,
    Override,
}

#[derive(Debug, Clone, Copy)]
struct Octave {
    period: f64,
    phase: f64,
    rule: OctaveRule,
    margin: f64,
}

/// The usual tempo range of a genre tag, by keyword.
fn genre_range(genre: Option<&str>) -> Option<(f64, f64)> {
    const TABLE: [(&[&str], f64, f64); 6] = [
        (
            &[
                "drum & bass",
                "drum and bass",
                "drum'n'bass",
                "drum n bass",
                "dnb",
                "d&b",
                "jungle",
            ],
            160.0,
            180.0,
        ),
        (&["dubstep"], 135.0, 145.0),
        (&["trance"], 125.0, 150.0),
        (&["techno"], 120.0, 150.0),
        (&["house"], 120.0, 130.0),
        (&["hip-hop", "hip hop", "hiphop", "rap"], 80.0, 100.0),
    ];
    let genre = genre?.to_lowercase();
    TABLE
        .iter()
        .find(|(keys, _, _)| keys.iter().any(|k| genre.contains(k)))
        .map(|&(_, lo, hi)| (lo, hi))
}

fn choose_octave(
    tempo: &Tempo,
    phase: f64,
    settings: &SolveSettings,
    onsets: &[TimedOnset],
    span: (f64, f64),
) -> Octave {
    let onset_times: Vec<f64> = onsets.iter().map(|o| o.time_s).collect();
    // Candidates slow to fast. Half tempo has two possible parities: keep the one on more onsets.
    let slow_phase = {
        let a = slot_hits(&onset_times, tempo.period * 2.0, phase, span, INLIER_S);
        let b = slot_hits(
            &onset_times,
            tempo.period * 2.0,
            phase + tempo.period,
            span,
            INLIER_S,
        );
        if b > a { phase + tempo.period } else { phase }
    };
    let candidates = [
        (tempo.period * 2.0, slow_phase),
        (tempo.period, phase),
        (tempo.period / 2.0, phase),
    ];
    let (lo, hi) = settings.bpm_range;
    let bpm_of = |period: f64| 60.0 / period;
    let in_range: Vec<(f64, f64)> = candidates
        .iter()
        .copied()
        .filter(|&(p, _)| (lo - 1e-9..=hi + 1e-9).contains(&bpm_of(p)))
        .collect();
    let octave = |(period, phase): (f64, f64), rule, margin| Octave {
        period,
        phase,
        rule,
        margin,
    };
    match in_range.len() {
        0 => {
            let distance = |p: f64| {
                let b = bpm_of(p);
                if b < lo { (lo / b).ln() } else { (b / hi).ln() }
            };
            let best = candidates
                .iter()
                .copied()
                .min_by(|a, b| distance(a.0).total_cmp(&distance(b.0)))
                .unwrap_or((tempo.period, phase));
            octave(best, OctaveRule::Range, 1.0)
        }
        1 => octave(in_range[0], OctaveRule::Range, 1.0),
        _ => {
            if let Some(tag) = settings.tag_bpm.filter(|t| *t > 0.0)
                && let Some(&c) = in_range
                    .iter()
                    .find(|c| (bpm_of(c.0) - tag).abs() / tag <= 0.02)
            {
                return octave(c, OctaveRule::Tag, 0.9);
            }
            if let Some((glo, ghi)) = genre_range(settings.genre.as_deref()) {
                let matches: Vec<(f64, f64)> = in_range
                    .iter()
                    .copied()
                    .filter(|c| (glo..=ghi).contains(&bpm_of(c.0)))
                    .collect();
                if matches.len() == 1 {
                    return octave(matches[0], OctaveRule::Genre, 0.8);
                }
            }
            // Kick density: move to the faster lattice while it keeps at least 75 % of the
            // slower lattice's density.
            let density = |c: (f64, f64)| slot_hits(&onset_times, c.0, c.1, span, INLIER_S);
            let mut chosen = in_range[0];
            let mut chosen_density = density(chosen);
            if chosen_density <= 0.0 {
                let model = in_range
                    .iter()
                    .copied()
                    .find(|c| (c.0 - tempo.period).abs() < 1e-12)
                    .unwrap_or(in_range[0]);
                return octave(model, OctaveRule::ModelOctave, AMBER);
            }
            let mut margin: f64 = 1.0;
            for &faster in &in_range[1..] {
                let d = density(faster);
                let ratio = d / chosen_density;
                margin = margin.min(if (0.6..0.9).contains(&ratio) {
                    (ratio - 0.75).abs() / 0.15
                } else {
                    1.0
                });
                if ratio >= 0.75 {
                    chosen = faster;
                    chosen_density = d;
                } else {
                    break;
                }
            }
            octave(chosen, OctaveRule::Density, margin)
        }
    }
}

/// Snaps to the nearest integer BPM when it lies within max(3 sigma, 0.002) of the fit.
fn snap_bpm(bpm: f64, sigma_bpm: f64) -> f64 {
    let round = bpm.round();
    if (bpm - round).abs() <= (3.0 * sigma_bpm).max(0.002) {
        round
    } else {
        bpm
    }
}

// ---------------------------------------------------------------------------------------------
// Bar 1

#[derive(Debug, Clone)]
struct Downbeat {
    r: usize,
    margin: f64,
    shifts: Vec<i8>,
}

impl Downbeat {
    fn pinned() -> Self {
        Self {
            r: 0,
            margin: 1.0,
            shifts: Vec::new(),
        }
    }
}

fn sigmoid(x: f32) -> f64 {
    1.0 / (1.0 + (-f64::from(x)).exp())
}

/// Which lattice position modulo `bar` is beat 1: model downbeat activation plus the accent
/// of the onsets on each position.
fn downbeat_phase(
    period: f64,
    phase: f64,
    bar: usize,
    span: (f64, f64),
    logits: &[f32],
    onsets: &[TimedOnset],
) -> Downbeat {
    if bar <= 1 {
        return Downbeat::pinned();
    }
    let mut levels: Vec<f64> = onsets.iter().map(|o| f64::from(o.level_db)).collect();
    let floor = if levels.is_empty() {
        -80.0
    } else {
        median(&mut levels) - 20.0
    };
    let window = INLIER_S.min(period / 4.0);
    let mut logit_sum = vec![0.0; bar];
    let mut accent_sum = vec![0.0; bar];
    let mut count = vec![0.0; bar];
    let first = ((span.0 - phase) / period).ceil();
    let last = ((span.1 - phase) / period).floor();
    let mut k = first;
    let mut o = 0;
    while k <= last {
        let t = phase + period * k;
        let r = rem_index(k, bar);
        let frame = t * MODEL_FPS;
        let logit = [-1.0, 0.0, 1.0]
            .iter()
            .filter_map(|d| frame_index(frame.round() + d).and_then(|i| logits.get(i)))
            .map(|&x| sigmoid(x))
            .fold(0.0, f64::max);
        while o < onsets.len() && onsets[o].time_s < t - window {
            o += 1;
        }
        let accent = onsets[o..]
            .iter()
            .take_while(|x| x.time_s <= t + window)
            .map(|x| f64::from(x.level_db))
            .fold(f64::NEG_INFINITY, f64::max);
        logit_sum[r] += logit;
        accent_sum[r] += if accent.is_finite() { accent } else { floor };
        count[r] += 1.0;
        k += 1.0;
    }
    let mean_accent = accent_sum
        .iter()
        .zip(&count)
        .filter(|(_, c)| **c > 0.0)
        .map(|(a, c)| a / c)
        .sum::<f64>()
        / count_f64(count.iter().filter(|c| **c > 0.0).count().max(1));
    let scores: Vec<f64> = (0..bar)
        .map(|r| {
            if count[r] > 0.0 {
                logit_sum[r] / count[r] + (accent_sum[r] / count[r] - mean_accent) / ACCENT_SCALE_DB
            } else {
                f64::NEG_INFINITY
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..bar).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    let best = order[0];
    let margin = if scores[order[1]].is_finite() {
        ((scores[best] - scores[order[1]]) / 0.3).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let shifts = order[1..]
        .iter()
        .map(|&r| signed_shift(r, best, bar))
        .collect();
    Downbeat {
        r: best,
        margin,
        shifts,
    }
}

/// The first lattice point at or after the first beat (to within half a period) that is beat 1.
fn first_downbeat(period: f64, phase: f64, bar: usize, r: usize, start: f64) -> f64 {
    let bar_f = count_f64(bar);
    let k_first = ((start - phase) / period).round();
    let offset = (count_f64(r) - k_first.rem_euclid(bar_f)).rem_euclid(bar_f);
    let mut k = k_first + offset;
    while phase + period * k < 0.0 {
        k += bar_f;
    }
    phase + period * k
}

/// The earliest significant rise within +/-40 ms of `t`.
fn snap_anchor(t: f64, onsets: &[TimedOnset]) -> Option<f64> {
    let near: Vec<&TimedOnset> = onsets
        .iter()
        .filter(|o| (o.time_s - t).abs() <= ANCHOR_WINDOW_S)
        .collect();
    let strongest = near
        .iter()
        .map(|o| o.rise_db)
        .fold(f32::NEG_INFINITY, f32::max);
    near.iter()
        .filter(|o| o.rise_db >= strongest - ANCHOR_SIGNIFICANCE_DB)
        .map(|o| o.time_s)
        .min_by(f64::total_cmp)
}

fn nearest_index(times: &[f64], t: f64) -> u32 {
    times
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()))
        .map_or(0, |(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
}

// ---------------------------------------------------------------------------------------------
// Fitness and verdict

#[derive(Debug, Clone, Copy)]
struct Fitness {
    p95_ms: f64,
    max_ms: f64,
    local_range_bpm: f64,
    local_range_significant: bool,
    drift_ppm: f64,
    drift_sigma_ppm: f64,
}

/// Residuals of the onsets (or, without enough onsets, the beats) against the grid anchored at
/// `anchor` with `period`, the local tempo range and the drift.
fn fitness(
    anchor: f64,
    period: f64,
    onsets: &[TimedOnset],
    beats: &[f64],
    span: (f64, f64),
) -> Fitness {
    let window = MATCH_WINDOW_S.min(period / 4.0);
    let match_events = |times: &mut dyn Iterator<Item = f64>| -> Vec<(f64, f64)> {
        let mut pairs: Vec<(f64, f64)> = Vec::new();
        for t in times {
            if t < span.0 - window || t > span.1 + window {
                continue;
            }
            let x = (t - anchor) / period;
            let j = x.round();
            let d = (x - j) * period;
            if d.abs() > window {
                continue;
            }
            match pairs.last_mut() {
                Some(last) if (last.0 - j).abs() < 0.5 => {
                    if d.abs() < (last.1 - (anchor + period * j)).abs() {
                        *last = (j, t);
                    }
                }
                _ => pairs.push((j, t)),
            }
        }
        pairs
    };
    let mut pairs = match_events(&mut onsets.iter().map(|o| o.time_s));
    if pairs.len() < MIN_BEATS {
        pairs = match_events(&mut beats.iter().copied());
    }
    let mut abs_ms: Vec<f64> = pairs
        .iter()
        .map(|&(j, t)| (t - (anchor + period * j)).abs() * 1000.0)
        .collect();
    abs_ms.sort_by(f64::total_cmp);
    let (p95_ms, max_ms) = if abs_ms.is_empty() {
        (0.0, 0.0)
    } else {
        let rank = (95 * abs_ms.len()).div_ceil(100).max(1);
        (abs_ms[rank - 1], abs_ms[abs_ms.len() - 1])
    };
    let (local_range_bpm, local_range_significant) = local_range(&pairs);
    let (drift_ppm, drift_sigma_ppm) = drift(&pairs);
    Fitness {
        p95_ms,
        max_ms,
        local_range_bpm,
        local_range_significant,
        drift_ppm,
        drift_sigma_ppm,
    }
}

/// Range of the tempo fitted over 128-slot windows (hop 32), and whether it exceeds four times
/// the windows' own standard error.
fn local_range(pairs: &[(f64, f64)]) -> (f64, bool) {
    let Some(&(first, _)) = pairs.first() else {
        return (0.0, false);
    };
    let last = pairs[pairs.len() - 1].0;
    let mut bpms = Vec::new();
    let mut sigmas = Vec::new();
    let mut start = first;
    loop {
        let window: Vec<(f64, f64)> = pairs
            .iter()
            .copied()
            .filter(|p| p.0 >= start && p.0 < start + LOCAL_WINDOW)
            .collect();
        if window.len() >= 16
            && let Some(line) = weighted_line(&window, &vec![1.0; window.len()])
            && line.slope > 0.0
        {
            let sigma = slope_sigma(&window, line);
            bpms.push(60.0 / line.slope);
            sigmas.push(60.0 * sigma / (line.slope * line.slope));
        }
        if start + LOCAL_WINDOW > last {
            break;
        }
        start += LOCAL_HOP;
    }
    if bpms.len() < 2 {
        return (0.0, false);
    }
    let range = bpms.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - bpms.iter().copied().fold(f64::INFINITY, f64::min);
    let noise = median(&mut sigmas);
    (range, range > 4.0 * noise)
}

/// Linear tempo change over the matched span in ppm, with its standard error, from a quadratic
/// fit `t = a + b u + c u^2` on `u` scaled to [-1, 1]: the period changes by `4 c / b` of itself
/// from start to end.
fn drift(pairs: &[(f64, f64)]) -> (f64, f64) {
    if pairs.len() < 16 {
        return (0.0, 0.0);
    }
    let j0 = pairs[0].0;
    let j1 = pairs[pairs.len() - 1].0;
    let half = (j1 - j0) / 2.0;
    if half <= 0.0 {
        return (0.0, 0.0);
    }
    let center = j0 + half;
    let rows: Vec<(f64, f64)> = pairs
        .iter()
        .map(|&(j, t)| ((j - center) / half, t))
        .collect();
    // Normal equations for [1, u, u^2].
    let mut m = [[0.0_f64; 3]; 3];
    let mut v = [0.0_f64; 3];
    for &(u, t) in &rows {
        let basis = [1.0, u, u * u];
        for a in 0..3 {
            for b in 0..3 {
                m[a][b] += basis[a] * basis[b];
            }
            v[a] += basis[a] * t;
        }
    }
    let Some(inv) = invert3(m) else {
        return (0.0, 0.0);
    };
    let coef: Vec<f64> = (0..3)
        .map(|a| (0..3).map(|b| inv[a][b] * v[b]).sum())
        .collect();
    let ss: f64 = rows
        .iter()
        .map(|&(u, t)| (t - (coef[0] + coef[1] * u + coef[2] * u * u)).powi(2))
        .sum();
    let sigma2 = ss / (count_f64(rows.len()) - 3.0).max(1.0);
    let (b, c) = (coef[1], coef[2]);
    if b <= 0.0 {
        return (0.0, 0.0);
    }
    let drift = 4.0 * c / b * 1e6;
    let sigma = 4.0 * (sigma2 * inv[2][2]).sqrt() / b * 1e6;
    (drift, sigma)
}

fn invert3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-300 {
        return None;
    }
    let mut inv = [[0.0; 3]; 3];
    for (r, row) in inv.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            // Cofactor of m[c][r] (transposed) over the determinant.
            let rows: Vec<usize> = (0..3).filter(|&i| i != c).collect();
            let cols: Vec<usize> = (0..3).filter(|&i| i != r).collect();
            let minor = m[rows[0]][cols[0]] * m[rows[1]][cols[1]]
                - m[rows[0]][cols[1]] * m[rows[1]][cols[0]];
            let sign = if (r + c) % 2 == 0 { 1.0 } else { -1.0 };
            *cell = sign * minor / det;
        }
    }
    Some(inv)
}

fn verdict(f: &Fitness) -> Verdict {
    let tempo_moves = (f.local_range_bpm > LOCAL_RANGE_BPM && f.local_range_significant)
        || (f.drift_ppm.abs() >= DRIFT_PPM && f.drift_ppm.abs() > 3.0 * f.drift_sigma_ppm);
    if tempo_moves {
        Verdict::Drifts
    } else if f.p95_ms < STATIC_P95_MS && f.max_ms < STATIC_MAX_MS {
        Verdict::Static
    } else if f.p95_ms < WARN_P95_MS && f.max_ms < WARN_MAX_MS {
        Verdict::StaticWarn
    } else {
        Verdict::Drifts
    }
}

// ---------------------------------------------------------------------------------------------
// Small numeric helpers

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        values[n / 2]
    } else {
        f64::midpoint(values[n / 2 - 1], values[n / 2])
    }
}

/// Counts in this module are far below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
fn count_f64(n: usize) -> f64 {
    n as f64
}

/// Grid indices in this module are far below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
fn index_f64(i: i64) -> f64 {
    i as f64
}

/// `k mod bar` for an integer-valued `k`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rem_index(k: f64, bar: usize) -> usize {
    k.rem_euclid(count_f64(bar)) as usize % bar
}

/// A frame index when `frame` is a non-negative integer value.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn frame_index(frame: f64) -> Option<usize> {
    (0.0..1e12).contains(&frame).then_some(frame as usize)
}

/// A shift in beats from `best` to `r`, in `(-bar/2, bar/2]`.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn signed_shift(r: usize, best: usize, bar: usize) -> i8 {
    let d = (r + bar - best) % bar;
    let signed = if d * 2 > bar {
        d as i64 - bar as i64
    } else {
        d as i64
    };
    signed.clamp(-128, 127) as i8
}

/// Reported statistics are display values; f32 keeps every meaningful digit.
#[allow(clippy::cast_possible_truncation)]
fn to_f32(x: f64) -> f32 {
    x as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_only_within_the_fit_noise() {
        assert!((snap_bpm(127.98, 0.001) - 127.98).abs() < 1e-12);
        assert!((snap_bpm(128.0015, 0.0001) - 128.0).abs() < 1e-12);
        assert!((snap_bpm(128.004, 0.002) - 128.0).abs() < 1e-12);
        assert!((snap_bpm(128.01, 0.002) - 128.01).abs() < 1e-12);
    }

    #[test]
    fn genre_keywords() {
        assert_eq!(genre_range(Some("Drum & Bass")), Some((160.0, 180.0)));
        assert_eq!(genre_range(Some("Progressive House")), Some((120.0, 130.0)));
        assert_eq!(genre_range(Some("Hip-Hop/Rap")), Some((80.0, 100.0)));
        assert_eq!(genre_range(Some("Türkçe Pop")), None);
        assert_eq!(genre_range(None), None);
    }

    #[test]
    fn signed_shifts_wrap_around_the_bar() {
        assert_eq!(signed_shift(1, 0, 4), 1);
        assert_eq!(signed_shift(2, 0, 4), 2);
        assert_eq!(signed_shift(3, 0, 4), -1);
        assert_eq!(signed_shift(0, 3, 4), 1);
        assert_eq!(signed_shift(4, 0, 9), 4);
        assert_eq!(signed_shift(5, 0, 9), -4);
    }

    #[test]
    fn irls_ignores_outliers() {
        let mut pairs: Vec<(f64, f64)> = (0..64)
            .map(|k| (f64::from(k), 0.3 + 0.5 * f64::from(k)))
            .collect();
        pairs[10].1 += 0.2;
        pairs[40].1 -= 0.15;
        let line = irls(&pairs).unwrap();
        assert!((line.slope - 0.5).abs() < 1e-9, "{line:?}");
        assert!((line.intercept - 0.3).abs() < 1e-9, "{line:?}");
    }

    #[test]
    fn inverse_of_a_symmetric_matrix() {
        let m = [[4.0, 1.0, 2.0], [1.0, 3.0, 0.5], [2.0, 0.5, 5.0]];
        let inv = invert3(m).unwrap();
        for (i, row) in m.iter().enumerate() {
            for (j, _) in inv.iter().enumerate() {
                let product: f64 = row
                    .iter()
                    .zip(&inv)
                    .map(|(a, inv_row)| a * inv_row[j])
                    .sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((product - expected).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn drift_recovers_a_linear_tempo_change() {
        // Period grows linearly by 500 ppm over 400 beats.
        let n = 400;
        let mut t = 1.0;
        let pairs: Vec<(f64, f64)> = (0..n)
            .map(|k| {
                let pair = (f64::from(k), t);
                t += 0.5 + 0.5 * 500e-6 * f64::from(k) / f64::from(n - 1);
                pair
            })
            .collect();
        let (ppm, sigma) = drift(&pairs);
        assert!((ppm - 500.0).abs() < 5.0, "{ppm} +/- {sigma}");
    }
}
