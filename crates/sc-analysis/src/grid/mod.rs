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
//!    The lattice then moves half a beat when the model's downbeat activation clearly sits
//!    there: a bass line on the off-beats can out-attack the kick in the kick band.
//! 3. **Octave**: inside the user's DJ-app BPM range first, then the file's BPM tag (2 %), the
//!    genre's usual range; when both octaves still fit the range the faster one is taken (what DJ
//!    apps do), marked uncertain unless the kicks also fill the faster lattice. Half tempo takes
//!    the parity the downbeat activation sits on, or else the one on more onsets.
//! 4. **Round BPM** only when the round value lies within max(3 sigma, 0.002) BPM of the fit: a
//!    true 127.98 exported as 128.00 drifts 56 ms over six minutes.
//! 5. **Bar 1**: the meter's downbeat phase from the model's downbeats (each votes for the bar
//!    position it lands on), its downbeat activation and, as a tie-breaker, the onset accents
//!    gives the lattice point; the anchor moves onto the earliest significant rise
//!    (within 6 dB of the strongest within +/-40 ms) only when that rise agrees with the global
//!    phase to 5 ms, so a single early or late kick never shifts the whole grid.
//! 6. **Fitness**: the running median of the attacks' residuals against the final grid (about two
//!    bars) gives the verdict from its P95 and maximum; local tempo over 128-beat windows and a
//!    quadratic drift are reported for the drift card; coverage, recall and the
//!    octave, downbeat and meter margins give a calibrated three-state confidence.

mod bar;
mod fitness;
mod num;
mod octave;
mod phase;
mod tempo;

use sc_core::analysis::{Alternatives, Confidence, Grid, Meter, Reason, Verdict};
use sc_core::{Bpm, SampleIndex, Seconds};

use bar::{Downbeat, downbeat_phase, first_bar_line, first_downbeat, nearest_index, snap_anchor};
use fitness::{Assessment, assess, fitness, line_residuals, verdict};
use num::{count_f64, index_f64, to_f32};
use octave::{TempoChoice, choose_tempo};
use phase::beat_phase;
use tempo::{clean_beats, fit_tempo};

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
/// Half-beat parity by the model's downbeat activation: the window around a lattice line
/// read, the least summed activation (about eight confident downbeats) the other lattice needs,
/// and how many times the current lattice's sum. Measured on hand-corrected grids: where an
/// off-beat bass pulled the grid half a beat off, the corrected lines held 3.5 to several
/// thousand times the activation of the analysed ones; on correct grids the other lattice held
/// at most 1.1 times.
const PARITY_WINDOW_S: f64 = 0.040;
const PARITY_MIN_ACTIVATION: f64 = 8.0;
const PARITY_RATIO: f64 = 3.0;
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
const LOCAL_WINDOW: f64 = 128.0;
const LOCAL_HOP: f64 = 32.0;
/// Attacks in the running median the verdict is judged on.
const SMOOTH_ATTACKS: usize = 8;
/// Confidence cut points.
const GREEN: f64 = 0.8;
const AMBER: f64 = 0.5;
/// Accents break ties between bar positions but never outvote the model's downbeat activation
/// (kick patterns such as the one-drop put the loudest kick on beat 3): their contribution is
/// `ACCENT_WEIGHT * tanh(difference / ACCENT_SCALE_DB)`.
const ACCENT_SCALE_DB: f64 = 6.0;
const ACCENT_WEIGHT: f64 = 0.25;
/// The downbeat activation read at the lattice counts half as much as the model's own downbeat
/// votes (the activation peaks can sit a frame or two off the attack-based lattice).
const LOGIT_WEIGHT: f64 = 0.5;

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
    /// Model downbeat times in seconds.
    pub downbeats_s: &'a [f64],
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
    /// A beat period chosen by the user (a tapped lattice), in seconds; replaces the octave
    /// choice but is snapped like it.
    pub tempo_period: Option<f64>,
    /// A bar line placed by the user, in seconds: bar 1 is the line of its lattice at or after
    /// the first beat (to within half a period); a line placed there is kept exactly.
    pub anchor_override_s: Option<f64>,
    /// Octave steps from the chosen tempo (the user's x2 / /2): +1 doubles it, -1 halves it.
    /// Ignored with `bpm_override`.
    pub octave_shift: i8,
    /// Bar positions to move beat 1 on from the solved one (the user's `1`-`n`). Ignored with
    /// `anchor_override_s`.
    pub downbeat_shift: u8,
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
            tempo_period: None,
            anchor_override_s: None,
            octave_shift: 0,
            downbeat_shift: 0,
        }
    }
}

/// A solved grid with the residual of every grid line, for the grid view's residual lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    /// The grid.
    pub grid: Grid,
    /// Residuals per grid line.
    pub lines: LineResiduals,
}

/// How far the attack nearest each grid line lands from it, lines counted from bar 1's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LineResiduals {
    /// Line index of `residuals_ms[0]` (bar 1's line is 0; earlier lines are negative). The
    /// first line is the first at or after the start of the file.
    pub first_line: i64,
    /// Signed residual per line in milliseconds (attack minus line); NaN where no attack lies
    /// within the match window. Lines after the last matched span are left out.
    pub residuals_ms: Vec<f32>,
    /// The line where the smoothed residual curve (the verdict's) is furthest from the grid.
    pub worst_line: Option<i64>,
    /// Attacks matched to a line.
    pub matched: u32,
    /// Attacks within the span the grid was judged on.
    pub attacks: u32,
}

/// Fits the grid; `None` when fewer than [`MIN_BEATS`] usable beats exist.
#[must_use]
pub fn solve(ev: &Evidence<'_>, settings: &SolveSettings, sample_rate: u32) -> Option<Grid> {
    solve_detailed(ev, settings, sample_rate).map(|s| s.grid)
}

/// As [`solve`], with the residual of every grid line.
#[must_use]
pub fn solve_detailed(
    ev: &Evidence<'_>,
    settings: &SolveSettings,
    sample_rate: u32,
) -> Option<Solved> {
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

    let (coverage, recall) = (tempo.coverage, tempo.recall);
    let phase = beat_phase(
        tempo.period,
        tempo.phase,
        tempo.alt_phase,
        span,
        onsets,
        ev.downbeat_logits,
    );

    let TempoChoice { base, chosen, bpm } =
        choose_tempo(&tempo, phase, settings, onsets, ev.downbeat_logits, span);
    let period = 60.0 / bpm;
    let bar = usize::from(settings.meter.beats_per_bar.max(1));
    // A slower lattice the user chose (half tempo, a slower tap) runs through the bar 1 of the
    // tempo it came from, not through whichever beat the model's phase falls on.
    let mut octave = chosen;
    if octave.period > base.period * 1.01 && settings.anchor_override_s.is_none() {
        let db = downbeat_phase(
            base.period,
            base.phase,
            bar,
            span,
            ev.downbeats_s,
            ev.downbeat_logits,
            onsets,
        );
        octave.phase = first_downbeat(base.period, base.phase, bar, db.r, span.0);
    }

    // Bar 1.
    let (anchor_s, downbeat) = if let Some(anchor) = settings.anchor_override_s {
        let bar_len = period * count_f64(bar);
        (
            first_bar_line(anchor, bar_len, span.0 - period / 2.0),
            Downbeat::pinned(),
        )
    } else {
        let db = downbeat_phase(
            period,
            octave.phase,
            bar,
            span,
            ev.downbeats_s,
            ev.downbeat_logits,
            onsets,
        )
        .shifted(settings.downbeat_shift, bar);
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

    let lines = line_residuals(&fit, anchor_s, period, span);
    let grid = Grid {
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
    };
    Some(Solved { grid, lines })
}

/// A grid from a typed tempo and a placed bar line alone, for tracks in which the model found
/// too few beats: bar 1 is the first bar line at or after the start of the file, residuals come
/// from the kick onsets (or the broadband ones without a kick), and the confidence stays amber
/// with the reason [`Reason::Manual`]. `None` for a tempo that is not positive.
#[must_use]
pub fn manual(
    bpm: f64,
    anchor_s: f64,
    meter: &Meter,
    ev: &Evidence<'_>,
    sample_rate: u32,
) -> Option<Solved> {
    if !(bpm.is_finite() && bpm > 0.0 && anchor_s.is_finite()) {
        return None;
    }
    let period = 60.0 / bpm;
    let bar_len = period * f64::from(meter.beats_per_bar.max(1));
    let anchor = first_bar_line(anchor_s, bar_len, 0.0);
    let onsets = if ev.kick_onsets.is_empty() {
        ev.broadband_onsets
    } else {
        ev.kick_onsets
    };
    let span = match (onsets.first(), onsets.last()) {
        (Some(first), Some(last)) => (first.time_s, last.time_s),
        _ => (anchor, anchor),
    };
    let fit = fitness(anchor, period, onsets, &[], span);
    let verdict = verdict(&fit);
    let mut reasons = vec![Reason::Manual];
    match verdict {
        Verdict::Static => {}
        Verdict::StaticWarn => reasons.push(Reason::Residuals),
        Verdict::Drifts => reasons.push(Reason::Drifts),
    }
    let lines = line_residuals(&fit, anchor, period, span);
    let grid = Grid {
        anchor: Seconds(anchor).to_sample_index(sample_rate),
        bpm: Bpm(bpm),
        meter: meter.clone(),
        meter_runner_up: None,
        first_downbeat_index: 0,
        phrase_len_bars: 8,
        segments: Vec::new(),
        residual_p95_ms: to_f32(fit.p95_ms),
        residual_max_ms: to_f32(fit.max_ms),
        local_bpm_range: to_f32(fit.local_range_bpm),
        drift_ppm: to_f32(fit.drift_ppm),
        verdict,
        confidence: Confidence::Amber,
        reasons,
        alternatives: Alternatives {
            octave_up: (bpm * 2.0 <= 400.0).then_some(Bpm(bpm * 2.0)),
            octave_down: (bpm / 2.0 >= 30.0).then_some(Bpm(bpm / 2.0)),
            downbeat_shift_beats: Vec::new(),
        },
    };
    Some(Solved { grid, lines })
}

/// The tempo fit of the model's beats alone: beat period and phase (seconds) and the span of
/// the usable beats. The meter estimator works on this lattice before the grid is solved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatFit {
    /// Seconds per model beat.
    pub period: f64,
    /// Time of lattice index 0, in seconds.
    pub phase: f64,
    /// A second phase the model followed for a sustained section (at least eight beats a
    /// quarter period or more away), such as a stretch on the off-beat.
    pub alt_phase: Option<f64>,
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
        alt_phase: tempo.alt_phase,
        span: (beats[0], beats[beats.len() - 1]),
    })
}

/// The global phase of the attacks on the model's beat lattice: the comb is tried around the
/// model's majority phase and, when the model itself followed another phase for a sustained
/// section, around that one too; the better one wins (the alternative needs a 20 % better score)
/// and is centred on the attacks' median offset. Without attacks the model's phase stands.
/// The lattice then moves half a beat when the model's downbeat activation (logits at 50 frames
/// per second) clearly sits there.
#[must_use]
pub fn onset_phase(fit: &BeatFit, onsets: &[TimedOnset], downbeat_logits: &[f32]) -> f64 {
    beat_phase(
        fit.period,
        fit.phase,
        fit.alt_phase,
        fit.span,
        onsets,
        downbeat_logits,
    )
}

/// Samples of a grid line: `anchor + i * 60 * sample_rate / bpm`, rounded.
#[must_use]
pub fn line_at(grid: &Grid, i: i64, sample_rate: u32) -> SampleIndex {
    let t = grid.anchor.to_seconds(sample_rate).0 + index_f64(i) * 60.0 / grid.bpm.0;
    Seconds(t).to_sample_index(sample_rate)
}
