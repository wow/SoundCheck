//! Steps 3 and 4, octave and round BPM: the octave choice, the user's tempo and octave steps,
//! and the snap to a round BPM within the fit's noise.

use super::phase::slot_hits;
use super::tempo::Tempo;
use super::{INLIER_S, SolveSettings, TimedOnset};

/// The tempo the solver settled on before the user's changes (`base`: their typed tempo, the
/// meter's fixed pulse or the octave choice), the one the grid uses (`chosen`: a tapped lattice
/// or the user's octave steps applied), and its BPM after the round-BPM snap.
pub(super) struct TempoChoice {
    pub(super) base: Octave,
    pub(super) chosen: Octave,
    pub(super) bpm: f64,
}

pub(super) fn choose_tempo(
    tempo: &Tempo,
    phase: f64,
    settings: &SolveSettings,
    onsets: &[TimedOnset],
    span: (f64, f64),
) -> TempoChoice {
    let user = |period| Octave {
        period,
        phase,
        rule: OctaveRule::Range,
        margin: 1.0,
    };
    let base = match settings.bpm_override {
        Some(bpm) if bpm > 0.0 => Octave {
            period: 60.0 / bpm,
            phase,
            rule: OctaveRule::Override,
            margin: 1.0,
        },
        _ => match settings.fixed_period {
            Some(period) if period > 0.0 => user(period),
            _ => choose_octave(tempo, phase, settings, onsets, span),
        },
    };
    let chosen = match settings.tempo_period {
        Some(period) if period > 0.0 && base.rule != OctaveRule::Override => user(period),
        _ => base,
    };
    let chosen = if settings.octave_shift != 0 && chosen.rule != OctaveRule::Override {
        shift_octave(chosen, settings.octave_shift)
    } else {
        chosen
    };
    let sigma_period = tempo.sigma_period * chosen.period / tempo.period;
    let fitted_bpm = 60.0 / chosen.period;
    let bpm = if chosen.rule == OctaveRule::Override {
        fitted_bpm
    } else {
        snap_bpm(
            fitted_bpm,
            60.0 * sigma_period / (chosen.period * chosen.period),
        )
    };
    TempoChoice { base, chosen, bpm }
}

/// The octave `steps` away from `o`: each step up halves the period, each step down doubles
/// it. The user chose it, so the octave is no longer in doubt; a slower lattice is moved onto
/// bar 1 by the caller.
pub(super) fn shift_octave(o: Octave, steps: i8) -> Octave {
    Octave {
        period: o.period * 2f64.powi(-i32::from(steps)),
        phase: o.phase,
        rule: o.rule,
        margin: 1.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OctaveRule {
    Range,
    Tag,
    Genre,
    Density,
    Override,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Octave {
    pub(super) period: f64,
    pub(super) phase: f64,
    pub(super) rule: OctaveRule,
    pub(super) margin: f64,
}

/// The usual tempo range of a genre tag, by keyword.
pub(super) fn genre_range(genre: Option<&str>) -> Option<(f64, f64)> {
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

pub(super) fn choose_octave(
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
            // Both octaves fit the range and nothing else decides: take the faster one, as DJ
            // apps do with such a range; it is certain only when the kicks also fill the faster
            // lattice (at least 75 % of the slower lattice's density).
            let density = |c: (f64, f64)| slot_hits(&onset_times, c.0, c.1, span, INLIER_S);
            let slow = in_range[0];
            let fast = in_range[in_range.len() - 1];
            let (d_slow, d_fast) = (density(slow), density(fast));
            let margin = if d_slow > 0.0 && d_fast / d_slow >= 0.75 {
                1.0
            } else {
                0.6
            };
            octave(fast, OctaveRule::Density, margin)
        }
    }
}

/// Snaps to the nearest integer BPM when it lies within max(3 sigma, 0.002) of the fit.
pub(super) fn snap_bpm(bpm: f64, sigma_bpm: f64) -> f64 {
    let round = bpm.round();
    if (bpm - round).abs() <= (3.0 * sigma_bpm).max(0.002) {
        round
    } else {
        bpm
    }
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
}
