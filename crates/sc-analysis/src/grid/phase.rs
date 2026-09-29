//! Step 2, phase: the comb over the onsets at the fitted period, centred on the attacks.

use super::bar::sigmoid;
use super::num::{count_f64, frame_index, median};
use super::{
    COMB_KERNEL_S, COMB_RANGE_S, COMB_STEPS, INLIER_S, MODEL_FPS, PARITY_MIN_ACTIVATION,
    PARITY_RATIO, PARITY_WINDOW_S, TimedOnset,
};

/// The beat phase: the comb over the attacks, then the half-beat parity by the model's
/// downbeat activation. A bass line on the off-beats can out-attack the kick in the kick band
/// and pull the comb half a beat off (through the model's alternative phase); the downbeat
/// activation, read before the model's post-processing snaps downbeats onto its beats, stays
/// on the bar.
pub(super) fn beat_phase(
    period: f64,
    phase: f64,
    alt_phase: Option<f64>,
    span: (f64, f64),
    onsets: &[TimedOnset],
    logits: &[f32],
) -> f64 {
    let combed = phase_from_onsets(period, phase, alt_phase, span, onsets);
    let other = combed + period / 2.0;
    if activation_prefers(period, combed, other, span, logits) {
        phase_from_onsets(period, other, None, span, onsets)
    } else {
        combed
    }
}

/// Whether the model's downbeat activation (logits at [`MODEL_FPS`]) puts the lattice of
/// `period` at phase `other` rather than `current`: summed over the lines within `span`, the
/// strongest activation within [`PARITY_WINDOW_S`] of each line must reach
/// [`PARITY_MIN_ACTIVATION`] and [`PARITY_RATIO`] times the sum on `current`'s lines.
pub(super) fn activation_prefers(
    period: f64,
    current: f64,
    other: f64,
    span: (f64, f64),
    logits: &[f32],
) -> bool {
    let reach = (PARITY_WINDOW_S.min(period / 4.0) * MODEL_FPS).floor();
    let activation = |phase: f64| -> f64 {
        let first = ((span.0 - phase) / period).ceil();
        let last = ((span.1 - phase) / period).floor();
        let mut sum = 0.0;
        let mut k = first;
        while k <= last {
            let frame = ((phase + period * k) * MODEL_FPS).round();
            let mut d = -reach;
            let mut best = 0.0_f64;
            while d <= reach {
                if let Some(&x) = frame_index(frame + d).and_then(|i| logits.get(i)) {
                    best = best.max(sigmoid(x));
                }
                d += 1.0;
            }
            sum += best;
            k += 1.0;
        }
        sum
    };
    let on_other = activation(other);
    on_other >= PARITY_MIN_ACTIVATION && on_other >= PARITY_RATIO * activation(current)
}

pub(super) fn phase_from_onsets(
    period: f64,
    phase: f64,
    alt_phase: Option<f64>,
    span: (f64, f64),
    onsets: &[TimedOnset],
) -> f64 {
    let (off_a, score_a) = comb_offset(onsets, period, phase, span);
    let mut coarse = phase + off_a;
    if let Some(alt) = alt_phase {
        let (off_b, score_b) = comb_offset(onsets, period, alt, span);
        if score_b > score_a * 1.2 {
            coarse = alt + off_b;
        }
    }
    coarse + median_offset(onsets, period, coarse, span)
}

/// Signed distance from `t` to the nearest lattice line.
pub(super) fn lattice_distance(t: f64, period: f64, phase: f64) -> f64 {
    let x = (t - phase) / period;
    (x - x.round()) * period
}

/// Offset (seconds) that best aligns the onsets with the lattice, within +/-40 ms, and its
/// score.
pub(super) fn comb_offset(
    onsets: &[TimedOnset],
    period: f64,
    phase: f64,
    span: (f64, f64),
) -> (f64, f64) {
    let relevant: Vec<&TimedOnset> = onsets
        .iter()
        .filter(|o| o.time_s >= span.0 - period && o.time_s <= span.1 + period)
        .collect();
    if relevant.is_empty() {
        return (0.0, 0.0);
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
    best
}

/// Median signed distance of the onsets within [`INLIER_S`] of the lattice: the comb finds the
/// peak region, this centres it (a plateau of evenly spread attacks has no single comb maximum).
pub(super) fn median_offset(
    onsets: &[TimedOnset],
    period: f64,
    phase: f64,
    span: (f64, f64),
) -> f64 {
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
pub(super) fn slot_hits(
    events: &[f64],
    period: f64,
    phase: f64,
    span: (f64, f64),
    window: f64,
) -> f64 {
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

#[cfg(test)]
mod tests;
