//! Step 6, fitness: residuals against the final grid, local tempo, drift, the verdict, the
//! residual lane and the calibrated confidence.

use sc_core::analysis::{Confidence, Reason, Verdict};

use super::num::{count_f64, line_count, line_index, line_slot, running_median, to_f32};
use super::tempo::weighted_slope;
use super::{
    AMBER, GREEN, LOCAL_HOP, LOCAL_WINDOW, LineResiduals, MATCH_WINDOW_S, MIN_BEATS, SHORT_BEATS,
    SMOOTH_ATTACKS, STATIC_MAX_MS, STATIC_P95_MS, SolveSettings, TimedOnset, WARN_MAX_MS,
    WARN_P95_MS,
};

#[derive(Debug, Clone)]
pub(super) struct Fitness {
    pub(super) p95_ms: f64,
    pub(super) max_ms: f64,
    pub(super) local_range_bpm: f64,
    pub(super) drift_ppm: f64,
    /// Matched (line index from the anchor, attack time) pairs, one per line.
    pub(super) pairs: Vec<(f64, f64)>,
    /// The running median of the pairs' residuals, in milliseconds.
    pub(super) smooth: Vec<f64>,
    /// Events within the judged span, matched or not.
    pub(super) attacks: usize,
}

/// Residuals of the onsets (or, without enough onsets, the beats) against the grid anchored at
/// `anchor` with `period`, the local tempo range and the drift.
pub(super) fn fitness(
    anchor: f64,
    period: f64,
    onsets: &[TimedOnset],
    beats: &[f64],
    span: (f64, f64),
) -> Fitness {
    let window = MATCH_WINDOW_S.min(period / 4.0);
    let match_events = |times: &mut dyn Iterator<Item = f64>| -> (Vec<(f64, f64)>, usize) {
        let mut pairs: Vec<(f64, f64)> = Vec::new();
        let mut within = 0;
        for t in times {
            if t < span.0 - window || t > span.1 + window {
                continue;
            }
            within += 1;
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
        (pairs, within)
    };
    let (mut pairs, mut attacks) = match_events(&mut onsets.iter().map(|o| o.time_s));
    if pairs.len() < MIN_BEATS && !beats.is_empty() {
        (pairs, attacks) = match_events(&mut beats.iter().copied());
    }
    // A static grid fails when the attacks drift away from it for a while, not when single
    // attacks scatter: judge the running median over SMOOTH_ATTACKS consecutive attacks
    // (about two bars).
    let residuals: Vec<f64> = pairs
        .iter()
        .map(|&(j, t)| (t - (anchor + period * j)) * 1000.0)
        .collect();
    let smooth = running_median(&residuals, SMOOTH_ATTACKS);
    let mut abs_ms: Vec<f64> = smooth.iter().map(|r| r.abs()).collect();
    abs_ms.sort_by(f64::total_cmp);
    let (p95_ms, max_ms) = if abs_ms.is_empty() {
        (0.0, 0.0)
    } else {
        let rank = (95 * abs_ms.len()).div_ceil(100).max(1);
        (abs_ms[rank - 1], abs_ms[abs_ms.len() - 1])
    };
    // Local tempo and drift from the same smoothed curve: stray attacks must not bend them.
    let smooth_pairs: Vec<(f64, f64)> = pairs
        .iter()
        .zip(&smooth)
        .map(|(&(j, _), r)| (j, anchor + period * j + r / 1000.0))
        .collect();
    let local_range_bpm = local_range(&smooth_pairs);
    let drift_ppm = drift(&smooth_pairs).0;
    Fitness {
        p95_ms,
        max_ms,
        local_range_bpm,
        drift_ppm,
        pairs,
        smooth,
        attacks,
    }
}

/// Range of the tempo fitted over 128-slot windows (hop 32), in BPM.
pub(super) fn local_range(pairs: &[(f64, f64)]) -> f64 {
    let Some(&(first, _)) = pairs.first() else {
        return 0.0;
    };
    let last = pairs[pairs.len() - 1].0;
    let mut bpms = Vec::new();
    let mut start = first;
    loop {
        let window: Vec<(f64, f64)> = pairs
            .iter()
            .copied()
            .filter(|p| p.0 >= start && p.0 < start + LOCAL_WINDOW)
            .collect();
        if window.len() >= 16
            && let Some(slope) = weighted_slope(&window, &vec![1.0; window.len()])
            && slope > 0.0
        {
            bpms.push(60.0 / slope);
        }
        if start + LOCAL_WINDOW > last {
            break;
        }
        start += LOCAL_HOP;
    }
    if bpms.len() < 2 {
        return 0.0;
    }
    bpms.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - bpms.iter().copied().fold(f64::INFINITY, f64::min)
}

/// Linear tempo change over the matched span in ppm, with its standard error, from a quadratic
/// fit `t = a + b u + c u^2` on `u` scaled to [-1, 1]: the period changes by `4 c / b` of itself
/// from start to end.
pub(super) fn drift(pairs: &[(f64, f64)]) -> (f64, f64) {
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

pub(super) fn invert3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
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

/// Whether one static grid fits, judged on the smoothed residual curve alone: a tempo change
/// that the curve does not show (a few hundred ppm over a song moves the grid by milliseconds)
/// does not matter to a DJ, and one that matters shows in the curve.
pub(super) fn verdict(f: &Fitness) -> Verdict {
    if f.p95_ms < STATIC_P95_MS && f.max_ms < STATIC_MAX_MS {
        Verdict::Static
    } else if f.p95_ms < WARN_P95_MS && f.max_ms < WARN_MAX_MS {
        Verdict::StaticWarn
    } else {
        Verdict::Drifts
    }
}

/// The residual lane: one value per grid line from the first line in the file to the end of
/// the judged span.
pub(super) fn line_residuals(
    fit: &Fitness,
    anchor: f64,
    period: f64,
    span: (f64, f64),
) -> LineResiduals {
    let window = MATCH_WINDOW_S.min(period / 4.0);
    let (lo, hi) = fit
        .pairs
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.0), hi.max(p.0))
        });
    let first = (-anchor / period).ceil().min(lo);
    let last = ((span.1 + window - anchor) / period).floor().max(hi);
    let len = if last >= first {
        line_count(last - first + 1.0)
    } else {
        0
    };
    let mut residuals_ms = vec![f32::NAN; len];
    for &(j, t) in &fit.pairs {
        if let Some(slot) = line_slot(j - first).and_then(|i| residuals_ms.get_mut(i)) {
            *slot = to_f32((t - (anchor + period * j)) * 1000.0);
        }
    }
    let worst_line = fit
        .smooth
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map(|(i, _)| line_index(fit.pairs[i].0));
    LineResiduals {
        first_line: line_index(first),
        residuals_ms,
        worst_line,
        matched: u32::try_from(fit.pairs.len()).unwrap_or(u32::MAX),
        attacks: u32::try_from(fit.attacks).unwrap_or(u32::MAX),
    }
}

/// The inputs of the confidence and the reason chips.
pub(super) struct Assessment<'a> {
    pub(super) coverage: f64,
    pub(super) recall: f64,
    pub(super) octave_margin: f64,
    pub(super) downbeat_margin: f64,
    pub(super) verdict: Verdict,
    pub(super) bpm: f64,
    pub(super) used_broadband: bool,
    pub(super) beats: usize,
    pub(super) settings: &'a SolveSettings,
}

/// Calibrated three-state confidence from the weakest term, and the chips explaining it.
pub(super) fn assess(a: &Assessment<'_>) -> (Confidence, Vec<Reason>) {
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

#[cfg(test)]
mod tests;
