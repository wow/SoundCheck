//! Step 2, phase: the comb over the onsets at the fitted period, centred on the attacks.

use super::num::{count_f64, median};
use super::{COMB_KERNEL_S, COMB_RANGE_S, COMB_STEPS, INLIER_S, TimedOnset};

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
