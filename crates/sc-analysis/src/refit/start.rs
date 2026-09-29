//! The start fit's window: where the music starts and how many beats of it one tempo holds.
//!
//! The window begins at the first model beat at or after the first attack that starts a run
//! ([`START_RUN_ATTACKS`] attacks within [`START_RUN_BEATS`] beats), so an intro without drums
//! is skipped even when a stray hit sits in it, and spans [`START_BEATS`] beats of the
//! whole-track tempo, [`START_BEATS_WIDE`] when those hold fewer than [`START_MIN_ATTACKS`]
//! attacks; when even that holds too few there is no start fit. A tempo change inside the window
//! would blend two tempi, so shorter windows ([`START_SHORTER`]) are tried too, each fitted on
//! its own beats. All of them are scored on the same stretch, the opening ([`START_OPENING_BEATS`]
//! beats, where a DJ mixes in), by their precision there: the share of its attacks that land on
//! the window's lattice ([`grid::lattice_precision`]). A fit blended across a tempo change
//! misses the opening; a breakdown, or attacks off the beat (an off-beat bass, hi-hats), cost
//! every window alike. A tempo change too small to push the opening's attacks off the lines
//! (under about 0.25 BPM of blend) still bends a longer fit: so, from the shortest window up, a
//! longer one is taken only while its tempo agrees with the shorter one's within
//! [`START_AGREE_SIGMAS`] standard errors of the shorter fit, when the shorter fit itself holds
//! the opening ([`START_TRUST_PRECISION`]; over a loose intro the longest window near the best
//! precision is taken), and while its opening precision is
//! within [`START_SHORTER_GAIN`] of the best. The longest window is preferred because more
//! beats give a steadier tempo: the model's beats sit on 20 ms frames, so 128 beats pin the tempo
//! to about 0.003 BPM where 32 beats leave about 0.025 (and a round BPM within three times that
//! is taken), and on real tracks whose tempo creeps 128 beats matched hand-set tempi to
//! 0.02 BPM where 64 beats missed by 0.05.

use sc_core::analysis::GridEvidence;

use super::grid_attacks;
use crate::grid::{self, BeatFit, TimedOnset};

/// Beats the start window spans at most, of the whole-track tempo: about a minute, the window
/// the verdict's local tempo is measured over.
pub const START_BEATS: u32 = 128;
/// Beats the window widens to when the first [`START_BEATS`] hold too few attacks.
pub const START_BEATS_WIDE: u32 = 192;
/// Fewest attacks the window must hold for a start fit.
pub const START_MIN_ATTACKS: usize = 32;
/// Shorter windows tried, in beats, when the tempo changes inside the longest one.
pub const START_SHORTER: [u32; 2] = [64, 32];
/// A longer window must give the tempo of the shorter one within this many of the shorter
/// fit's standard errors (the model's beats sit on 20 ms frames, so a steady tempo agrees).
pub const START_AGREE_SIGMAS: f64 = 3.0;
/// A shorter window overrules a longer one's tempo only when its own fit holds this share of
/// the opening's attacks: over a sparse or loose intro the few model beats of a short window
/// give a tempo no better than the longer fit's.
pub const START_TRUST_PRECISION: f64 = 0.8;
/// Beats of the opening every window is scored on: the shortest window.
pub const START_OPENING_BEATS: u32 = 32;
/// A longer window wins unless a shorter one holds this much more of the opening's attacks.
pub const START_SHORTER_GAIN: f64 = 0.1;
/// Attacks that make a run: the music starts at the first attack followed by this many (itself
/// included) within [`START_RUN_BEATS`] beats.
pub const START_RUN_ATTACKS: usize = 8;
/// Beats a run spans at most: the attack density the widest window needs
/// ([`START_MIN_ATTACKS`] in [`START_BEATS_WIDE`] beats), so a sparse track that still gets a
/// start fit also has a run.
pub const START_RUN_BEATS: u32 = 48;

/// The chosen start window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartWindow {
    /// First beat of the window, in seconds from the start of the file.
    pub from_s: f64,
    /// End of the window, in seconds.
    pub to_s: f64,
    /// Length in beats of the whole-track tempo (one of [`START_BEATS_WIDE`], [`START_BEATS`],
    /// [`START_SHORTER`]).
    pub beats: u32,
    /// Share (0 to 1) of the opening's attacks ([`START_OPENING_BEATS`] beats) within 20 ms of
    /// a line of the window's fit: what the window was chosen by.
    pub precision: f64,
    /// Share (0 to 1) of the window's lines of its own fit holding an attack within 20 ms (what
    /// the fit choice shows for each grid).
    pub line_share: f64,
}

/// The start fit's window on `evidence`; `None` when the model found too few beats, no run of
/// attacks starts the music, or the start holds too few attacks for a start fit.
#[must_use]
pub fn start_window_s(evidence: &GridEvidence) -> Option<StartWindow> {
    let beats_s: Vec<f64> = evidence.beats_s.iter().map(|&b| f64::from(b)).collect();
    let whole = grid::fit_beats(&beats_s)?;
    choose(&beats_s, &whole, &grid_attacks(evidence))
}

/// [`start_window_s`] from the model's beats, their whole-track fit and the attacks.
pub(super) fn choose(
    beats_s: &[f64],
    whole: &BeatFit,
    attacks: &[TimedOnset],
) -> Option<StartWindow> {
    let period = whole.period;
    // A model beat can sit a frame before the attack it marks.
    let slack = period / 4.0;
    let mut times: Vec<f64> = attacks
        .iter()
        .map(|o| o.time_s)
        .filter(|t| t.is_finite())
        .collect();
    times.sort_by(f64::total_cmp);
    let run_s = f64::from(START_RUN_BEATS) * period;
    let first_attack = times.iter().enumerate().find_map(|(i, &t)| {
        let run = times[i..].partition_point(|&u| u <= t + run_s);
        (run >= START_RUN_ATTACKS).then_some(t)
    })?;
    let from_s = beats_s
        .iter()
        .copied()
        .filter(|&t| t.is_finite() && t >= first_attack - slack && t >= whole.span.0)
        .min_by(f64::total_cmp)?;
    let end = |beats: u32| from_s + f64::from(beats) * period;
    let held = |beats: u32| {
        attacks
            .iter()
            .filter(|o| o.time_s >= from_s - slack && o.time_s <= end(beats))
            .count()
    };
    let longest = [START_BEATS, START_BEATS_WIDE]
        .into_iter()
        .find(|&b| held(b) >= START_MIN_ATTACKS)?;
    // Shortest first.
    let candidates = START_SHORTER
        .into_iter()
        .rev()
        .chain(std::iter::once(longest))
        .map(|beats| {
            let to_s = end(beats);
            let fit = grid::fit_beats_within(beats_s, Some((from_s, to_s))).unwrap_or(*whole);
            let phase = grid::onset_phase(&fit, attacks, &[]);
            let window = StartWindow {
                from_s,
                to_s,
                beats,
                precision: grid::lattice_precision(
                    fit.period,
                    phase,
                    attacks,
                    from_s - slack,
                    end(START_OPENING_BEATS),
                ),
                line_share: grid::lattice_share(fit.period, phase, attacks, from_s, to_s),
            };
            (window, fit)
        });
    let candidates: Vec<(StartWindow, BeatFit)> = candidates.collect();
    let best = candidates
        .iter()
        .map(|(w, _)| w.precision)
        .fold(f64::NEG_INFINITY, f64::max);
    // Grow from the shortest window. Once a window holds the opening, a longer one must hold it
    // too and keep its tempo; until then (a loose intro), the longest near the best is taken.
    let mut chosen: Option<(StartWindow, BeatFit)> = None;
    for (window, fit) in candidates {
        let near_best = window.precision >= best - START_SHORTER_GAIN;
        match &chosen {
            Some((held, shorter)) if held.precision >= START_TRUST_PRECISION => {
                let agrees = (fit.period - shorter.period).abs()
                    <= START_AGREE_SIGMAS * shorter.sigma_period;
                if !(near_best && agrees) {
                    break;
                }
                chosen = Some((window, fit));
            }
            _ if near_best => chosen = Some((window, fit)),
            _ => {}
        }
    }
    chosen.map(|(window, _)| window)
}
