//! Step 5, bar 1: the downbeat position within the bar, the lattice point it gives, the snap
//! onto the attack, and a placed bar line counted back to the first bar.

use super::num::{count_f64, frame_index, median, rem_index, signed_shift};
use super::{
    ACCENT_SCALE_DB, ACCENT_WEIGHT, ANCHOR_SIGNIFICANCE_DB, ANCHOR_WINDOW_S, INLIER_S,
    LOGIT_WEIGHT, MODEL_FPS, TimedOnset,
};

/// The bar line of `t`'s lattice (bars of `bar_len` seconds) at or after `start`, and never
/// before the start of the file.
pub(super) fn first_bar_line(t: f64, bar_len: f64, start: f64) -> f64 {
    let n = ((t - start) / bar_len).floor();
    let mut line = if n == 0.0 { t } else { t - n * bar_len };
    while line < 0.0 {
        line += bar_len;
    }
    line
}

#[derive(Debug, Clone)]
pub(super) struct Downbeat {
    pub(super) r: usize,
    pub(super) margin: f64,
    pub(super) shifts: Vec<i8>,
}

impl Downbeat {
    pub(super) fn pinned() -> Self {
        Self {
            r: 0,
            margin: 1.0,
            shifts: Vec::new(),
        }
    }

    /// Beat 1 moved `by` bar positions on (the user's choice, so no longer in doubt); the other
    /// candidates stay in their order, as shifts from the new position.
    pub(super) fn shifted(self, by: u8, bar: usize) -> Self {
        if by == 0 || bar <= 1 {
            return self;
        }
        let r = (self.r + usize::from(by)) % bar;
        let shifts = std::iter::once(self.r)
            .chain(self.shifts.iter().map(|&s| position(self.r, s, bar)))
            .filter(|&p| p != r)
            .map(|p| signed_shift(p, r, bar))
            .collect();
        Self {
            r,
            margin: 1.0,
            shifts,
        }
    }
}

/// The bar position `shift` beats from `r`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
pub(super) fn position(r: usize, shift: i8, bar: usize) -> usize {
    (r as i64 + i64::from(shift)).rem_euclid(bar as i64) as usize
}

pub(super) fn sigmoid(x: f32) -> f64 {
    1.0 / (1.0 + (-f64::from(x)).exp())
}

/// Which lattice position modulo `bar` is beat 1: model downbeat activation plus the accent
/// of the onsets on each position.
#[allow(clippy::too_many_arguments)]
pub(super) fn downbeat_phase(
    period: f64,
    phase: f64,
    bar: usize,
    span: (f64, f64),
    downbeats: &[f64],
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
    // Votes: each model downbeat counts for the bar position it lands on.
    let mut votes = vec![0.0; bar];
    let vote_window = (period / 4.0).min(0.1);
    for &d in downbeats {
        if d < span.0 - vote_window || d > span.1 + vote_window {
            continue;
        }
        let x = (d - phase) / period;
        let k = x.round();
        if (x - k).abs() * period <= vote_window {
            votes[rem_index(k, bar)] += 1.0;
        }
    }
    let total_votes: f64 = votes.iter().sum();
    let vote_share = |r: usize| {
        if total_votes >= 4.0 {
            votes[r] / total_votes
        } else {
            0.0
        }
    };
    let scores: Vec<f64> = (0..bar)
        .map(|r| {
            if count[r] > 0.0 {
                vote_share(r)
                    + LOGIT_WEIGHT * logit_sum[r] / count[r]
                    + ACCENT_WEIGHT
                        * ((accent_sum[r] / count[r] - mean_accent) / ACCENT_SCALE_DB).tanh()
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
pub(super) fn first_downbeat(period: f64, phase: f64, bar: usize, r: usize, start: f64) -> f64 {
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
pub(super) fn snap_anchor(t: f64, onsets: &[TimedOnset]) -> Option<f64> {
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

pub(super) fn nearest_index(times: &[f64], t: f64) -> u32 {
    times
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()))
        .map_or(0, |(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
}
