//! Step 1, tempo: the joint robust fit of one period over the model's phase-consistent beat
//! segments.

use super::num::{count_f64, median};
use super::{HUBER_S, MIN_BEATS, REJECT_S};

#[derive(Debug, Clone, Copy)]
pub(super) struct Tempo {
    pub(super) period: f64,
    /// Phase of the largest group of segments that agree (the model's majority phase).
    pub(super) phase: f64,
    /// The second group's phase, when it is sustained and a quarter period or more away.
    pub(super) alt_phase: Option<f64>,
    pub(super) sigma_period: f64,
    /// Fraction of the beats that fit the shared period in their segment.
    pub(super) coverage: f64,
    /// Fitting beats per lattice slot over the span (capped at 1).
    pub(super) recall: f64,
}

/// A spacing more than this fraction of a period away from a whole number of periods is a
/// phase jump (the model moved to the off-beat or back); a new segment starts there.
pub(super) const PHASE_JUMP: f64 = 0.25;

/// Sorted, finite, non-negative beats without double detections (closer than 30 % of the
/// median spacing to the previous kept beat).
pub(super) fn clean_beats(beats: &[f64]) -> Vec<f64> {
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

/// One period shared by every phase-consistent segment of the beats, each with its own phase:
/// a half-beat jump of the model (to the off-beat and back) then cannot bend the tempo.
pub(super) fn fit_tempo(beats: &[f64]) -> Option<Tempo> {
    if beats.len() < MIN_BEATS {
        return None;
    }
    let mut increments: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    let spacing = median(&mut increments);
    if spacing <= 0.0 {
        return None;
    }
    let segs = segments(beats, spacing);
    let first = joint_irls(&segs)?;
    // Again with the fitted period, which indexes long gaps more reliably than the median
    // spacing (the model's spacings are quantised to 20 ms).
    let segs = segments(beats, first.slope);
    let joint = joint_irls(&segs)?;
    if joint.slope <= 0.0 {
        return None;
    }
    let period = joint.slope;

    // Inliers per segment and the residual spread.
    let (mut inliers, mut ss, mut skk) = (0usize, 0.0, 0.0);
    let mut segment_weight = Vec::with_capacity(segs.len());
    for (seg, &ic) in segs.iter().zip(&joint.intercepts) {
        let fitting: Vec<(f64, f64)> = seg
            .iter()
            .copied()
            .filter(|&(k, t)| ic.is_finite() && (t - (ic + period * k)).abs() <= REJECT_S)
            .collect();
        segment_weight.push(count_f64(fitting.len()));
        if fitting.is_empty() {
            continue;
        }
        inliers += fitting.len();
        let mk = fitting.iter().map(|p| p.0).sum::<f64>() / count_f64(fitting.len());
        for &(k, t) in &fitting {
            ss += (t - (ic + period * k)).powi(2);
            skk += (k - mk) * (k - mk);
        }
    }
    let dof = count_f64(inliers).max(3.0) - count_f64(segs.len()) - 1.0;
    let sigma_period = if skk > 0.0 && dof > 0.0 {
        (ss / dof).sqrt() / skk.sqrt()
    } else {
        f64::INFINITY
    };

    // The phase most segments agree on (circularly, within the rejection distance).
    let wrap = |x: f64| x - period * (x / period).floor();
    let phases: Vec<f64> = joint.intercepts.iter().map(|&ic| wrap(ic)).collect();
    let mut best = (0.0, -1.0);
    for (i, &candidate) in phases.iter().enumerate() {
        if !joint.intercepts[i].is_finite() {
            continue;
        }
        let support: f64 = phases
            .iter()
            .zip(&segment_weight)
            .filter(|(p, _)| {
                let d = (*p - candidate).abs();
                d.min(period - d) <= REJECT_S
            })
            .map(|(_, w)| w)
            .sum();
        if support > best.1 {
            best = (candidate, support);
        }
    }
    let near_start = |p: f64| p + period * ((beats[0] - p) / period).round();
    let phase = near_start(best.0);
    let circular = |a: f64, b: f64| {
        let d = (a - b).abs();
        d.min(period - d)
    };
    let mut alt = (0.0, 0.0);
    for (i, &candidate) in phases.iter().enumerate() {
        if !joint.intercepts[i].is_finite() || circular(candidate, best.0) < period / 4.0 {
            continue;
        }
        let support: f64 = phases
            .iter()
            .zip(&segment_weight)
            .filter(|(p, _)| circular(**p, candidate) <= REJECT_S)
            .map(|(_, w)| w)
            .sum();
        if support > alt.1 {
            alt = (candidate, support);
        }
    }
    let alt_phase = (alt.1 >= count_f64(MIN_BEATS)).then(|| near_start(alt.0));
    let slots = ((beats[beats.len() - 1] - beats[0]) / period).round() + 1.0;
    Some(Tempo {
        period,
        phase,
        alt_phase,
        sigma_period,
        coverage: count_f64(inliers) / count_f64(beats.len()),
        recall: (count_f64(inliers) / slots).min(1.0),
    })
}

/// Splits the beats into runs whose spacings stay near whole numbers of `spacing` and indexes
/// each run from zero; a beat less than half a spacing after the last kept one is skipped.
pub(super) fn segments(beats: &[f64], spacing: f64) -> Vec<Vec<(f64, f64)>> {
    let mut all = Vec::new();
    let mut current = vec![(0.0, beats[0])];
    let (mut k, mut last) = (0.0, beats[0]);
    for &t in &beats[1..] {
        let x = (t - last) / spacing;
        let steps = x.round();
        if steps < 1.0 {
            continue;
        }
        if (x - steps).abs() > PHASE_JUMP {
            all.push(std::mem::replace(&mut current, vec![(0.0, t)]));
            k = 0.0;
            last = t;
            continue;
        }
        k += steps;
        current.push((k, t));
        last = t;
    }
    all.push(current);
    all
}

#[derive(Debug, Clone)]
pub(super) struct Joint {
    pub(super) slope: f64,
    /// One per segment; NaN for a segment without weight.
    pub(super) intercepts: Vec<f64>,
}

/// Weighted least squares of `t = intercept_s + slope * k` with one intercept per segment.
pub(super) fn joint_fit(segs: &[Vec<(f64, f64)>], weights: &[Vec<f64>]) -> Option<Joint> {
    let (mut num, mut den) = (0.0, 0.0);
    let mut means = Vec::with_capacity(segs.len());
    for (seg, ws) in segs.iter().zip(weights) {
        let sw: f64 = ws.iter().sum();
        if sw <= 0.0 {
            means.push(None);
            continue;
        }
        let mk = seg.iter().zip(ws).map(|(p, w)| p.0 * w).sum::<f64>() / sw;
        let mt = seg.iter().zip(ws).map(|(p, w)| p.1 * w).sum::<f64>() / sw;
        for (&(k, t), &w) in seg.iter().zip(ws) {
            num += w * (k - mk) * (t - mt);
            den += w * (k - mk) * (k - mk);
        }
        means.push(Some((mk, mt)));
    }
    if den <= 0.0 {
        return None;
    }
    let slope = num / den;
    let intercepts = means
        .iter()
        .map(|m| m.map_or(f64::NAN, |(mk, mt)| mt - slope * mk))
        .collect();
    Some(Joint { slope, intercepts })
}

/// Iteratively reweighted [`joint_fit`]: Huber weights, then hard rejection beyond 40 ms.
pub(super) fn joint_irls(segs: &[Vec<(f64, f64)>]) -> Option<Joint> {
    let mut weights: Vec<Vec<f64>> = segs.iter().map(|s| vec![1.0; s.len()]).collect();
    let mut joint = joint_fit(segs, &weights)?;
    for iteration in 0..8 {
        for ((seg, ws), &ic) in segs.iter().zip(weights.iter_mut()).zip(&joint.intercepts) {
            for (w, &(k, t)) in ws.iter_mut().zip(seg) {
                let r = if ic.is_finite() {
                    (t - (ic + joint.slope * k)).abs()
                } else {
                    f64::INFINITY
                };
                *w = if iteration >= 2 && r > REJECT_S {
                    0.0
                } else if r <= HUBER_S {
                    1.0
                } else {
                    HUBER_S / r
                };
            }
        }
        joint = joint_fit(segs, &weights)?;
    }
    Some(joint)
}

/// Slope of the weighted least-squares line through `(k, t)` pairs.
pub(super) fn weighted_slope(pairs: &[(f64, f64)], weights: &[f64]) -> Option<f64> {
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
    (skk > 0.0).then(|| skt / skk)
}

#[cfg(test)]
mod tests;
