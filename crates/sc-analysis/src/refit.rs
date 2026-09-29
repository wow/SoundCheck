//! The grid from cached evidence, with or without a user's edit.
//!
//! The analysis itself is a refit with no edit: the engine stores the [`GridEvidence`] it
//! solved from and calls [`refit`] on it, so solving the stored evidence again reproduces the
//! analysed grid exactly, and an edit changes only what it overrides:
//!
//! - **meter**: replaces the estimated one (x/8 meters take the estimator's pulse as their
//!   beat); the estimated meter becomes the runner-up;
//! - **typed BPM**: used exactly;
//! - **tapped BPM**: the lattice of the fitted tempo (x3, x2, x3/2, x1, x2/3, x1/2, x1/3)
//!   within 4 % of the tap, else the kicks' own tempo within 4 % (or twice or half of it), else
//!   the tap itself;
//! - **octave**: steps from the chosen tempo;
//! - **bar line**: bar 1 is that line's lattice point at or after the first beat;
//! - **beat 1**: which beat of the solved bar starts the bar;
//! - **fit**: `start` fits tempo, phase and meter to the start of the music ([`start`]: up to
//!   128 beats from the first beat with an attack, fewer when the tempo already changes inside
//!   them), for a track whose tempo changes: a static grid fitted to the whole track takes its
//!   middle and misses the start, where a DJ mixes in and where bar 1 is judged. It stays one
//!   static grid judged over the whole track, so such a track still drifts. The other overrides
//!   apply on top; without enough attacks at the start there is no start fit and the whole
//!   track is fitted.
//!
//! Every choice the user made counts as certain, so its margin no longer lowers the
//! confidence; coverage, recall and the verdict still do. A track with too few beats gets a
//! grid only from a typed BPM and a placed bar line ([`grid::manual`]). A refit takes well under
//! 5 ms on a six-minute track (`benches/grid.rs`), so the grid view recomputes on every edit.

use sc_core::analysis::{
    BeatUnit, EDIT_BPM_LIMITS, GridEdit, GridEvidence, GridFit, Meter, OnsetList, TagHints,
};
use sc_core::{Bpm, SampleIndex};

use crate::grid::{self, BeatFit, Evidence, SolveSettings, Solved, TimedOnset};
use crate::meter;

pub mod start;

pub use start::{StartWindow, start_window_s};

/// A tap selects a lattice when it lies within this fraction of the lattice's tempo.
const TAP_TOLERANCE: f64 = 0.04;
/// Multiples of the fitted beat period a tap may select.
const TAP_RATIOS: [f64; 7] = [3.0, 2.0, 1.5, 1.0, 2.0 / 3.0, 0.5, 1.0 / 3.0];

/// What a grid is solved against besides the evidence and the edit.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The user's DJ-app BPM range.
    pub bpm_range: (Bpm, Bpm),
    /// The file's tag hints (BPM, genre, title and artist for the meter prior).
    pub tags: &'a TagHints,
    /// The file's native sample rate, which grid positions count in.
    pub sample_rate: u32,
}

/// Longest time a placed bar line may lie from the start of the file.
const MAX_ANCHOR_S: f64 = 24.0 * 3600.0;

/// The grid for `evidence` under `edit`, with its per-line residuals; `None` when the model
/// found too few beats and the edit does not give both a BPM and a bar line, and when the edit
/// is outside its limits ([`GridEdit::validate`], a bar line within 24 hours) or would give a
/// tempo outside [`EDIT_BPM_LIMITS`].
#[must_use]
pub fn refit(evidence: &GridEvidence, ctx: &Context<'_>, edit: &GridEdit) -> Option<Solved> {
    if edit.is_empty() {
        return solve(evidence, ctx, edit);
    }
    edit.validate().ok()?;
    if edit
        .anchor
        .is_some_and(|a| a.to_seconds(ctx.sample_rate).0 > MAX_ANCHOR_S)
    {
        return None;
    }
    let (lo, hi) = EDIT_BPM_LIMITS;
    solve(evidence, ctx, edit).filter(|s| (lo..=hi).contains(&s.grid.bpm.0))
}

fn solve(evidence: &GridEvidence, ctx: &Context<'_>, edit: &GridEdit) -> Option<Solved> {
    let beats_s: Vec<f64> = evidence.beats_s.iter().map(|&b| f64::from(b)).collect();
    let downbeats_s: Vec<f64> = evidence.downbeats_s.iter().map(|&b| f64::from(b)).collect();
    let kick = timed(&evidence.kick_onsets);
    let broadband = timed(&evidence.broadband_onsets);
    let ev = Evidence {
        beats_s: &beats_s,
        downbeats_s: &downbeats_s,
        downbeat_logits: &evidence.downbeat_logits_50fps,
        kick_onsets: &kick,
        broadband_onsets: &broadband,
    };
    let anchor_s = edit.anchor.map(|a| a.to_seconds(ctx.sample_rate).0);
    let Some(whole) = grid::fit_beats(&beats_s) else {
        let meter = edit.meter.clone().unwrap_or_else(Meter::four_four);
        return grid::manual(edit.bpm?.0, anchor_s?, &meter, &ev, ctx.sample_rate);
    };
    // The start fit reads the meter, the tap's lattices and the grid from the start window's
    // beats alone, so all three agree.
    let fit_window_s = if edit.fit == GridFit::Start {
        start::choose(&beats_s, &whole, attacks(beats_s.len(), &kick, &broadband))
            .map(|w| (w.from_s, w.to_s))
    } else {
        None
    };
    let fit = match fit_window_s {
        Some(window) => grid::fit_beats_within(&beats_s, Some(window)).unwrap_or(whole),
        None => whole,
    };

    // The meter is read on the attacks' phase, not the model's: trackers sometimes follow the
    // off-beat for whole sections.
    let anchor_onsets = if kick.len() * 4 >= beats_s.len() {
        &kick
    } else {
        &broadband
    };
    let phase = grid::onset_phase(&fit, anchor_onsets, &evidence.downbeat_logits_50fps);
    let hint = [&ctx.tags.genre, &ctx.tags.title, &ctx.tags.artist]
        .iter()
        .filter_map(|s| s.as_deref())
        .collect::<Vec<_>>()
        .join(" ");
    let estimate = meter::estimate(
        fit.period,
        phase,
        fit.span,
        &kick,
        &broadband,
        &evidence.downbeat_logits_50fps,
        &hint,
    );
    let (meter, meter_margin, runner_up, fixed_period) = match &edit.meter {
        Some(chosen) => (
            chosen.clone(),
            1.0,
            if *chosen == estimate.meter {
                estimate.runner_up.clone()
            } else {
                Some(estimate.meter.clone())
            },
            (chosen.unit == BeatUnit::Eighth).then_some(estimate.pulse_period),
        ),
        None => (
            estimate.meter.clone(),
            estimate.margin,
            estimate.runner_up.clone(),
            (estimate.meter.unit == BeatUnit::Eighth).then_some(estimate.unit_period),
        ),
    };
    let (tempo_period, bpm_override) = match (edit.bpm, edit.tempo_hint) {
        (Some(bpm), _) => (None, Some(bpm.0)),
        (None, Some(tap)) => match tapped_period(tap.0, &fit, &kick) {
            Some(period) => (Some(period), None),
            None => (None, Some(tap.0)),
        },
        (None, None) => (None, None),
    };
    let settings = SolveSettings {
        bpm_range: (ctx.bpm_range.0.0, ctx.bpm_range.1.0),
        tag_bpm: ctx.tags.bpm.map(|b| b.0),
        genre: ctx.tags.genre.clone(),
        meter,
        meter_margin,
        fixed_period,
        bpm_override,
        tempo_period,
        anchor_override_s: anchor_s,
        octave_shift: edit.octave,
        downbeat_shift: edit.downbeat_shift,
        fit_window_s,
    };
    grid::solve_detailed(&ev, &settings, ctx.sample_rate).map(|mut solved| {
        solved.grid.meter_runner_up = runner_up;
        solved
    })
}

/// The beat period a tap at `tap_bpm` selects: a lattice of the fitted tempo, else of the kicks'
/// own tempo; `None` when neither lies within [`TAP_TOLERANCE`].
fn tapped_period(tap_bpm: f64, fit: &BeatFit, kick: &[TimedOnset]) -> Option<f64> {
    if !(tap_bpm.is_finite() && tap_bpm > 0.0) {
        return None;
    }
    let nearest = |periods: &mut dyn Iterator<Item = f64>| {
        periods
            .map(|p| (p, (60.0 / p / tap_bpm).ln().abs()))
            .filter(|&(_, d)| d <= (1.0 + TAP_TOLERANCE).ln())
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p)
    };
    nearest(&mut TAP_RATIOS.iter().map(|r| fit.period * r)).or_else(|| {
        let times: Vec<f64> = kick.iter().map(|o| o.time_s).collect();
        let kicks = grid::fit_beats(&times)?;
        nearest(&mut [2.0, 1.0, 0.5].iter().map(|r| kicks.period * r))
    })
}

/// The attacks the solver matches grid lines against, on the timeline: the kick-band onsets,
/// or the broadband ones when the kick band is nearly silent (fewer than one kick per four
/// beats) and broadband ones exist.
#[must_use]
pub fn grid_attacks(evidence: &GridEvidence) -> Vec<TimedOnset> {
    let kick = timed(&evidence.kick_onsets);
    let broadband = timed(&evidence.broadband_onsets);
    attacks(evidence.beats_s.len(), &kick, &broadband).to_vec()
}

/// [`grid_attacks`]'s choice, for `beats` model beats and onsets already on the timeline.
fn attacks<'a>(
    beats: usize,
    kick: &'a [TimedOnset],
    broadband: &'a [TimedOnset],
) -> &'a [TimedOnset] {
    if kick.len() * 4 >= beats || broadband.is_empty() {
        kick
    } else {
        broadband
    }
}

/// Onsets on the timeline in seconds.
#[must_use]
pub fn timed(list: &OnsetList) -> Vec<TimedOnset> {
    list.frames
        .iter()
        .zip(&list.rise_db)
        .zip(&list.level_db)
        .map(|((&frame, &rise_db), &level_db)| TimedOnset {
            time_s: SampleIndex(u64::from(frame)).to_seconds(list.sample_rate).0,
            rise_db,
            level_db,
        })
        .collect()
}
