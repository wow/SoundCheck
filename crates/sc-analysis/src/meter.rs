//! Meter estimation: 4/4, 3/4, 6/8, 9/8 (2+2+2+3 and 3+2+2+2), 5/8, 7/8 and 10/8, from the
//! accent pattern at the finest stable pulse.
//!
//! Beat trackers trained mostly on 4/4 material read odd meters as 4/4: on 9/8 aksak their beats
//! sit on the eighth-note pulse and their downbeats fall anywhere. So the meter is estimated
//! here, from evidence the model does not interpret:
//!
//! 1. **Pulse**: the model's beat period, or its half or third when attacks fill those
//!    subdivisions (at least 40 % of the slots); a model period faster than 200 per minute is
//!    already the eighth-note pulse.
//! 2. **Accent per pulse**: the loudest attack's level, whether a kick lands on it, and the
//!    model's downbeat activation, each standardised over the track and summed (weights 1.0,
//!    0.7, 0.7).
//! 3. **Score per meter**: the accent profile folded at the bar length is correlated with the
//!    meter's template (1.0 on the bar start, 0.6 on the other group starts, 0 elsewhere) at
//!    every rotation; the best rotation's correlation plus the accent sequence's
//!    autocorrelation at the bar length plus a prior (4/4 favoured by 0.2, odd meters penalised
//!    by 0.1 unless the genre or title suggests Turkish, Balkan, Greek or Arabic music, where
//!    they gain 0.15).
//! 4. The best meter, the runner-up and their margin (difference of scores, clamped to 0..1)
//!    go to the grid, whose downbeat phase is then solved at that bar length.
//!
//! At the eighth-note pulse 4/4 and 3/4 are folded as 2+2+2+2 and 2+2+2, so every candidate is
//! compared on the same pulse.

use sc_core::analysis::{BeatUnit, Meter};

use crate::grid::TimedOnset;

/// Frames per second of the model's activations.
const MODEL_FPS: f64 = 50.0;
/// Fraction of subdivision slots that must hold an attack for the pulse to subdivide.
const SUBDIVISION_DENSITY: f64 = 0.4;
/// A model beat period faster than this (per minute) is taken as the eighth-note pulse.
const EIGHTH_PULSE_PER_MIN: f64 = 200.0;
/// Template weights.
const BAR_START: f64 = 1.0;
const GROUP_START: f64 = 0.6;
/// Accent component weights.
const W_LEVEL: f64 = 1.0;
const W_KICK: f64 = 0.7;
const W_DOWNBEAT: f64 = 0.7;
/// Priors.
const PRIOR_FOUR_FOUR: f64 = 0.2;
const PRIOR_ODD: f64 = -0.1;
const PRIOR_ODD_REGIONAL: f64 = 0.15;

/// Keywords in a genre or title that raise the prior of odd meters.
const REGIONAL: [&str; 14] = [
    "türk",
    "turk",
    "halk",
    "balkan",
    "roman",
    "greek",
    "rebetiko",
    "laïko",
    "laiko",
    "arab",
    "oriental",
    "çiftetelli",
    "ciftetelli",
    "karşılama",
];

/// The estimator's result.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterEstimate {
    /// The chosen meter.
    pub meter: Meter,
    /// The meter that scored second.
    pub runner_up: Option<Meter>,
    /// Score difference to the runner-up, clamped to 0..1 (a confidence term).
    pub margin: f64,
    /// Period of the pulse the meter was estimated on, in seconds.
    pub pulse_period: f64,
    /// Period of one grid beat at `meter.unit`: two pulses for 4/4 and 3/4 at the eighth
    /// level, one pulse otherwise.
    pub unit_period: f64,
}

/// One candidate at the pulse level.
struct Candidate {
    meter: Meter,
    /// Pulses per group at the pulse level.
    groups: Vec<u8>,
    /// Pulses per grid beat of `meter`.
    pulses_per_unit: f64,
}

fn candidates(eighth_level: bool) -> Vec<Candidate> {
    let quarter = |meter: Meter| {
        let groups = if eighth_level {
            vec![2; meter.grouping.len()]
        } else {
            meter.grouping.clone()
        };
        Candidate {
            meter,
            groups,
            pulses_per_unit: if eighth_level { 2.0 } else { 1.0 },
        }
    };
    let mut list = vec![quarter(Meter::four_four()), quarter(Meter::three_four())];
    if eighth_level {
        for meter in [
            Meter::six_eight(),
            Meter::nine_eight_aksak(),
            Meter::nine_eight_long_first(),
            Meter::five_eight(),
            Meter::seven_eight(),
            Meter::ten_eight(),
        ] {
            list.push(Candidate {
                groups: meter.grouping.clone(),
                meter,
                pulses_per_unit: 1.0,
            });
        }
    }
    list
}

/// Estimates the meter from the model's beat lattice (`period`, `phase`), the attacks and the
/// downbeat activations over `span`. `hint` is the genre and title text used for the prior.
#[must_use]
pub fn estimate(
    period: f64,
    phase: f64,
    span: (f64, f64),
    kick: &[TimedOnset],
    broadband: &[TimedOnset],
    downbeat_logits: &[f32],
    hint: &str,
) -> MeterEstimate {
    let attacks: Vec<TimedOnset> = merged(kick, broadband);
    let (pulse, eighth_level) = pulse_period(period, phase, span, &attacks);
    let accents = accents(pulse, phase, span, kick, &attacks, downbeat_logits);
    let regional = {
        let text = hint.to_lowercase();
        REGIONAL.iter().any(|k| text.contains(k))
    };
    let mut scored: Vec<(f64, Candidate)> = candidates(eighth_level)
        .into_iter()
        .map(|c| (score(&accents, &c, regional), c))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let margin = match scored.get(1) {
        Some(second) => (scored[0].0 - second.0).clamp(0.0, 1.0),
        None => 1.0,
    };
    let mut ranked = scored.into_iter();
    let Some((_, best)) = ranked.next() else {
        return MeterEstimate {
            meter: Meter::four_four(),
            runner_up: None,
            margin: 0.0,
            pulse_period: pulse,
            unit_period: period,
        };
    };
    let runner_up = ranked.next().map(|(_, c)| c.meter);
    MeterEstimate {
        unit_period: pulse * best.pulses_per_unit,
        meter: best.meter,
        runner_up,
        margin,
        pulse_period: pulse,
    }
}

fn merged(kick: &[TimedOnset], broadband: &[TimedOnset]) -> Vec<TimedOnset> {
    let mut all: Vec<TimedOnset> = kick.iter().chain(broadband).copied().collect();
    all.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    all
}

/// The finest stable pulse and whether it is the eighth-note level.
fn pulse_period(period: f64, phase: f64, span: (f64, f64), attacks: &[TimedOnset]) -> (f64, bool) {
    if 60.0 / period >= EIGHTH_PULSE_PER_MIN {
        return (period, true);
    }
    let times: Vec<f64> = attacks.iter().map(|o| o.time_s).collect();
    let halves = subdivision_density(&times, period, phase, span, &[0.5]);
    let thirds = subdivision_density(&times, period, phase, span, &[1.0 / 3.0, 2.0 / 3.0]);
    if thirds >= SUBDIVISION_DENSITY && thirds > halves {
        (period / 3.0, true)
    } else if halves >= SUBDIVISION_DENSITY {
        (period / 2.0, true)
    } else {
        (period, false)
    }
}

/// Fraction of the given in-beat positions (fractions of a period) that hold an attack.
fn subdivision_density(
    times: &[f64],
    period: f64,
    phase: f64,
    span: (f64, f64),
    fractions: &[f64],
) -> f64 {
    let window = (period / 8.0).min(0.020);
    let first = ((span.0 - phase) / period).ceil();
    let last = ((span.1 - phase) / period).floor() - 1.0;
    if last < first {
        return 0.0;
    }
    let (mut slots, mut hits) = (0.0, 0.0);
    let mut k = first;
    while k <= last {
        for f in fractions {
            let t = phase + period * (k + f);
            slots += 1.0;
            if has_event(times, t, window) {
                hits += 1.0;
            }
        }
        k += 1.0;
    }
    hits / slots
}

fn has_event(sorted: &[f64], t: f64, window: f64) -> bool {
    let i = sorted.partition_point(|&x| x < t - window);
    sorted.get(i).is_some_and(|&x| x <= t + window)
}

/// Standardised accent per pulse slot over the span.
fn accents(
    pulse: f64,
    phase: f64,
    span: (f64, f64),
    kick: &[TimedOnset],
    attacks: &[TimedOnset],
    logits: &[f32],
) -> Vec<f64> {
    let window = (pulse / 4.0).min(0.025);
    let kick_times: Vec<f64> = kick.iter().map(|o| o.time_s).collect();
    let first = ((span.0 - phase) / pulse).ceil();
    let last = ((span.1 - phase) / pulse).floor();
    let mut level = Vec::new();
    let mut kicked = Vec::new();
    let mut downbeat = Vec::new();
    let floor = {
        let mut l: Vec<f64> = attacks.iter().map(|o| f64::from(o.level_db)).collect();
        l.sort_by(f64::total_cmp);
        l.get(l.len() / 2).map_or(-80.0, |m| m - 20.0)
    };
    let mut k = first;
    while k <= last {
        let t = phase + pulse * k;
        let lo = attacks.partition_point(|o| o.time_s < t - window);
        let loudest = attacks[lo..]
            .iter()
            .take_while(|o| o.time_s <= t + window)
            .map(|o| f64::from(o.level_db))
            .fold(f64::NEG_INFINITY, f64::max);
        level.push(if loudest.is_finite() { loudest } else { floor });
        kicked.push(if has_event(&kick_times, t, window) {
            1.0
        } else {
            0.0
        });
        let frame = (t * MODEL_FPS).round();
        let activation = [-1.0, 0.0, 1.0]
            .iter()
            .filter_map(|d| frame_index(frame + d).and_then(|i| logits.get(i)))
            .map(|&x| 1.0 / (1.0 + (-f64::from(x)).exp()))
            .fold(0.0, f64::max);
        downbeat.push(activation);
        k += 1.0;
    }
    let level = standardise(&level);
    let kicked = standardise(&kicked);
    let downbeat = standardise(&downbeat);
    (0..level.len())
        .map(|i| W_LEVEL * level[i] + W_KICK * kicked[i] + W_DOWNBEAT * downbeat[i])
        .collect()
}

/// Zero mean, unit variance; a constant sequence becomes all zeros.
fn standardise(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let n = count_f64(values.len());
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    if var <= 1e-12 {
        return vec![0.0; values.len()];
    }
    let sd = var.sqrt();
    values.iter().map(|v| (v - mean) / sd).collect()
}

fn score(accents: &[f64], c: &Candidate, regional: bool) -> f64 {
    let bar: usize = c.groups.iter().map(|&g| usize::from(g)).sum();
    if accents.len() < 2 * bar {
        return f64::NEG_INFINITY;
    }
    let mut template = vec![0.0; bar];
    let mut position = 0;
    for (i, &g) in c.groups.iter().enumerate() {
        template[position] = if i == 0 { BAR_START } else { GROUP_START };
        position += usize::from(g);
    }
    let best_rotation = (0..bar)
        .map(|r| {
            let mut profile = vec![0.0; bar];
            let mut count = vec![0.0; bar];
            for (j, &a) in accents.iter().enumerate() {
                let q = (j + bar - r % bar) % bar;
                profile[q] += a;
                count[q] += 1.0;
            }
            for (p, n) in profile.iter_mut().zip(&count) {
                if *n > 0.0 {
                    *p /= n;
                }
            }
            correlation(&profile, &template)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    // Odd: the x/8 meters other than 6/8 (5/8, 7/8, 9/8, 10/8).
    let odd = c.meter.unit == BeatUnit::Eighth && c.meter.beats_per_bar != 6;
    let prior = if c.meter == Meter::four_four() {
        if regional { 0.0 } else { PRIOR_FOUR_FOUR }
    } else if odd {
        if regional {
            PRIOR_ODD_REGIONAL
        } else {
            PRIOR_ODD
        }
    } else {
        0.0
    };
    best_rotation + autocorrelation(accents, bar) + prior
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let a = standardise(a);
    let b = standardise(b);
    let n = count_f64(a.len());
    a.iter().zip(&b).map(|(x, y)| x * y).sum::<f64>() / n
}

/// Normalised autocorrelation of a standardised sequence at `lag`.
fn autocorrelation(a: &[f64], lag: usize) -> f64 {
    if a.len() <= lag {
        return 0.0;
    }
    let n = count_f64(a.len() - lag);
    a.iter().zip(&a[lag..]).map(|(x, y)| x * y).sum::<f64>() / n
}

/// Counts here are far below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
fn count_f64(n: usize) -> f64 {
    n as f64
}

/// A frame index when `frame` is a non-negative integer value.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn frame_index(frame: f64) -> Option<usize> {
    (0.0..1e12).contains(&frame).then_some(frame as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standardise_and_correlate() {
        let s = standardise(&[1.0, 2.0, 3.0]);
        assert!((s.iter().sum::<f64>()).abs() < 1e-12);
        assert!((correlation(&[1.0, 0.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]) - 1.0).abs() < 1e-12);
        assert_eq!(standardise(&[2.0; 4]), vec![0.0; 4]);
    }

    #[test]
    fn quarter_level_candidates_are_only_the_simple_meters() {
        let list = candidates(false);
        assert_eq!(list.len(), 2);
        let eighth = candidates(true);
        assert_eq!(eighth.len(), 8);
        assert_eq!(eighth[0].groups, vec![2, 2, 2, 2]);
        assert_eq!(eighth[3].groups, vec![2, 2, 2, 3]);
    }
}
