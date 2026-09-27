//! Refits with the user's edits on synthetic evidence shaped like the cached model output: beats
//! and downbeats on the model's 20 ms frames, downbeat activations, and kick onsets as frames at
//! the analysis rate with accents. Each fix must do exactly what it says: BPM to 1e-9, bar 1
//! within 1 ms (or one sample where a line is placed), grouping exact.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)] // test arithmetic on small counts; exact tempos are intended

use sc_analysis::grid::Solved;
use sc_analysis::refit::{Context, refit};
use sc_core::analysis::{
    BeatUnit, Confidence, Grid, GridEdit, GridEvidence, Meter, OnsetList, Reason, TagHints, Verdict,
};
use sc_core::{Bpm, SampleIndex, Seconds};

const SR: u32 = 44_100;
const ANALYSIS_RATE: u32 = 22_050;
const FIRST: f64 = 0.5;

/// Solved results compare equal, the residual lane's NaN gaps included.
fn assert_same(a: &Solved, b: &Solved) {
    assert_eq!(a.grid, b.grid);
    let bits = |s: &Solved| {
        s.lines
            .residuals_ms
            .iter()
            .map(|r| r.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(a), bits(b));
    assert_eq!(
        (
            a.lines.first_line,
            a.lines.worst_line,
            a.lines.matched,
            a.lines.attacks
        ),
        (
            b.lines.first_line,
            b.lines.worst_line,
            b.lines.matched,
            b.lines.attacks
        )
    );
}

/// Evidence of `bars` bars of `groups` (pulses per group) at `pulse` seconds per pulse. Every
/// pulse has an attack, group starts are louder, bar starts loudest; the model's beats fall
/// every `model_every` pulses and its downbeats on bar starts.
fn evidence(groups: &[u8], pulse: f64, bars: usize, model_every: usize) -> GridEvidence {
    let bar: usize = groups.iter().map(|&g| usize::from(g)).sum();
    let mut starts = vec![false; bar];
    let mut pos = 0;
    for &g in groups {
        starts[pos] = true;
        pos += usize::from(g);
    }
    let seconds = FIRST + pulse * (bar * bars) as f64 + 1.0;
    let mut ev = GridEvidence {
        downbeat_logits_50fps: vec![-6.0; (seconds * 50.0).ceil() as usize + 2],
        kick_onsets: OnsetList {
            sample_rate: ANALYSIS_RATE,
            ..OnsetList::default()
        },
        broadband_onsets: OnsetList {
            sample_rate: ANALYSIS_RATE,
            ..OnsetList::default()
        },
        ..GridEvidence::default()
    };
    for j in 0..bar * bars {
        let t = FIRST + pulse * j as f64;
        let q = j % bar;
        let level = if q == 0 {
            0.0
        } else if starts[q] {
            -4.0
        } else {
            -10.0
        };
        let frame = (t * f64::from(ANALYSIS_RATE)).round() as u32;
        for list in [&mut ev.broadband_onsets, &mut ev.kick_onsets] {
            if list.sample_rate == 0 {
                continue;
            }
            list.frames.push(frame);
            list.rise_db.push(25.0);
            list.level_db.push(level);
        }
        if !starts[q] {
            // Only group starts carry a kick.
            ev.kick_onsets.frames.pop();
            ev.kick_onsets.rise_db.pop();
            ev.kick_onsets.level_db.pop();
        }
        if j % model_every == 0 {
            let model_t = ((t * 50.0).round() / 50.0) as f32;
            ev.beats_s.push(model_t);
            if q == 0 {
                ev.downbeats_s.push(model_t);
            }
            ev.downbeat_logits_50fps[(t * 50.0).round() as usize] = if q == 0 { 3.0 } else { -3.0 };
        }
    }
    ev
}

/// A 4/4 track at `bpm` with a kick on every beat, 64 bars.
fn four_four(bpm: f64) -> GridEvidence {
    evidence(&[1, 1, 1, 1], 60.0 / bpm, 64, 1)
}

/// `ev` without its first `beats` model beats and the attacks before the next one: the track
/// then starts on another beat than bar 1.
fn without_first_beats(mut ev: GridEvidence, beats: usize) -> GridEvidence {
    let start = ev.beats_s[beats] - 0.01;
    ev.beats_s.drain(..beats);
    ev.downbeats_s.retain(|&d| d >= start);
    let cut = (f64::from(start) * f64::from(ANALYSIS_RATE)) as u32;
    for list in [&mut ev.kick_onsets, &mut ev.broadband_onsets] {
        let keep = list
            .frames
            .iter()
            .position(|&f| f >= cut)
            .unwrap_or(list.frames.len());
        list.frames.drain(..keep);
        list.rise_db.drain(..keep);
        list.level_db.drain(..keep);
    }
    ev
}

/// Whether `t` lies on one of `grid`'s bar lines, within 1 ms.
fn on_bar_line(grid: &Grid, t: f64) -> bool {
    let bar = f64::from(grid.meter.beats_per_bar) * 60.0 / grid.bpm.0;
    let off = (t - anchor_s(grid)).rem_euclid(bar);
    off.min(bar - off) <= 0.001
}

fn fit(ev: &GridEvidence, edit: &GridEdit) -> Solved {
    let tags = TagHints::default();
    let ctx = Context {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        tags: &tags,
        sample_rate: SR,
    };
    refit(ev, &ctx, edit).expect("a grid")
}

fn anchor_s(g: &Grid) -> f64 {
    g.anchor.to_seconds(SR).0
}

fn sample(t: f64) -> SampleIndex {
    Seconds(t).to_sample_index(SR)
}

#[test]
fn x2_and_half_double_and_halve_the_tempo_on_the_same_bar_lines() {
    for bpm in [124.0, 127.98, 96.5] {
        let ev = four_four(bpm);
        let base = fit(&ev, &GridEdit::default()).grid;
        assert!((base.bpm.0 - bpm).abs() < 0.005, "{bpm}: {}", base.bpm.0);
        let up = fit(
            &ev,
            &GridEdit {
                octave: 1,
                ..GridEdit::default()
            },
        )
        .grid;
        // Exactly double, or snapped to the round value the analysis would have chosen at that
        // octave (96.50026 x2 = 193.0005 snaps to 193.00): the same to two decimals either way.
        assert!(
            (up.bpm.0 - base.bpm.0 * 2.0).abs() <= 0.002,
            "{bpm}: x2 {}",
            up.bpm.0
        );
        assert_eq!(
            format!("{:.2}", up.bpm.0),
            format!("{:.2}", base.bpm.0 * 2.0)
        );
        assert!(
            (anchor_s(&up) - anchor_s(&base)).abs() <= 0.001,
            "{bpm}: {up:?}"
        );
        let down = fit(
            &ev,
            &GridEdit {
                octave: -1,
                ..GridEdit::default()
            },
        )
        .grid;
        assert!(
            (down.bpm.0 - base.bpm.0 / 2.0).abs() <= 0.002,
            "{bpm}: /2 {}",
            down.bpm.0
        );
        // Half tempo has bars twice as long: bar 1 is on one of the analysed bar lines.
        let old_bar = 4.0 * 60.0 / base.bpm.0;
        let off = (anchor_s(&down) - anchor_s(&base)).rem_euclid(old_bar);
        assert!(
            off.min(old_bar - off) <= 0.001,
            "{bpm}: /2 bar 1 off by {off}"
        );
    }
}

#[test]
fn slower_lattices_keep_bar_one_whichever_beat_the_track_starts_on() {
    for bpm in [124.0, 127.98, 174.0] {
        for dropped in 0..4 {
            let ev = without_first_beats(four_four(bpm), dropped);
            let base = fit(&ev, &GridEdit::default()).grid;
            assert!(
                on_bar_line(&base, FIRST),
                "{bpm}/{dropped}: the analysis keeps bar 1 ({:.4})",
                anchor_s(&base)
            );
            let halved = fit(
                &ev,
                &GridEdit {
                    octave: -1,
                    ..GridEdit::default()
                },
            )
            .grid;
            assert!(
                on_bar_line(&base, anchor_s(&halved)),
                "{bpm}/{dropped}: /2 puts bar 1 at {:.4}",
                anchor_s(&halved)
            );
            let tapped = fit(
                &ev,
                &GridEdit {
                    tempo_hint: Some(Bpm(base.bpm.0 / 2.0 * 1.01)),
                    ..GridEdit::default()
                },
            )
            .grid;
            assert!(
                (tapped.bpm.0 - base.bpm.0 / 2.0).abs() < 0.005,
                "{}",
                tapped.bpm.0
            );
            assert!(
                on_bar_line(&base, anchor_s(&tapped)),
                "{bpm}/{dropped}: a half-tempo tap puts bar 1 at {:.4}",
                anchor_s(&tapped)
            );
        }
    }
}

#[test]
fn edits_outside_the_limits_give_no_grid() {
    let ev = four_four(124.0);
    let tags = TagHints::default();
    let ctx = Context {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        tags: &tags,
        sample_rate: SR,
    };
    for edit in [
        GridEdit {
            bpm: Some(Bpm(1e7)),
            ..GridEdit::default()
        },
        GridEdit {
            bpm: Some(Bpm(f64::NAN)),
            ..GridEdit::default()
        },
        GridEdit {
            tempo_hint: Some(Bpm(0.5)),
            ..GridEdit::default()
        },
        GridEdit {
            octave: 12,
            ..GridEdit::default()
        },
        GridEdit {
            anchor: Some(SampleIndex(u64::MAX)),
            ..GridEdit::default()
        },
        GridEdit {
            meter: Some(Meter {
                beats_per_bar: 9,
                unit: BeatUnit::Eighth,
                grouping: vec![2, 2],
            }),
            ..GridEdit::default()
        },
    ] {
        assert!(refit(&ev, &ctx, &edit).is_none(), "{edit:?}");
    }
    // Two octaves down (31 BPM) is still a tempo; three (15.5 BPM) are not, and neither are
    // three up from 174 (1392 BPM).
    let steps = |octave| GridEdit {
        octave,
        ..GridEdit::default()
    };
    assert!((fit(&ev, &steps(-2)).grid.bpm.0 - 31.0).abs() < 1e-9);
    assert!(refit(&ev, &ctx, &steps(-3)).is_none());
    assert!(refit(&four_four(174.0), &ctx, &steps(3)).is_none());
    assert!(
        refit(&ev, &ctx, &steps(3)).is_some(),
        "992 BPM is inside the limits"
    );
}

#[test]
fn a_user_octave_clears_the_octave_doubt() {
    // 85 BPM kicks: 170 is taken inside 70-180 but only every other slot has a kick.
    let ev = four_four(85.0);
    let base = fit(&ev, &GridEdit::default()).grid;
    assert!((base.bpm.0 - 170.0).abs() < 1e-9, "{}", base.bpm.0);
    assert!(
        base.reasons.contains(&Reason::OctaveMargin),
        "{:?}",
        base.reasons
    );
    let halved = fit(
        &ev,
        &GridEdit {
            octave: -1,
            ..GridEdit::default()
        },
    )
    .grid;
    assert!((halved.bpm.0 - 85.0).abs() < 1e-9);
    assert!(
        !halved.reasons.contains(&Reason::OctaveMargin),
        "{:?}",
        halved.reasons
    );
    assert_eq!(halved.confidence, Confidence::Green, "{:?}", halved.reasons);
}

#[test]
fn beat_one_keys_move_bar_one_by_exactly_k_beats() {
    let ev = four_four(124.0);
    let base = fit(&ev, &GridEdit::default()).grid;
    let period = 60.0 / base.bpm.0;
    for k in 1..4_u8 {
        let g = fit(
            &ev,
            &GridEdit {
                downbeat_shift: k,
                ..GridEdit::default()
            },
        )
        .grid;
        let want = anchor_s(&base) + f64::from(k) * period;
        assert!(
            (anchor_s(&g) - want).abs() <= 0.001,
            "{k}: {:.4} vs {want:.4}",
            anchor_s(&g)
        );
        assert_eq!(g.bpm, base.bpm);
        assert!(
            !g.reasons.contains(&Reason::DownbeatMargin),
            "{k}: {:?}",
            g.reasons
        );
        assert!(!g.alternatives.downbeat_shift_beats.contains(&0), "{k}");
        assert!(
            g.alternatives
                .downbeat_shift_beats
                .contains(&(-i8::try_from(k).unwrap()))
                || g.alternatives
                    .downbeat_shift_beats
                    .contains(&(4 - i8::try_from(k).unwrap())),
            "{k}: the analysed bar 1 stays an alternative: {:?}",
            g.alternatives.downbeat_shift_beats
        );
    }
}

#[test]
fn a_placed_bar_line_is_kept_and_counted_from_the_first_beat() {
    let ev = four_four(124.0);
    let base = fit(&ev, &GridEdit::default()).grid;
    let bar_len = 4.0 * 60.0 / base.bpm.0;
    // The user nudges bar 1 by +3 ms: kept to the sample.
    let placed = sample(anchor_s(&base) + 0.003);
    let g = fit(
        &ev,
        &GridEdit {
            anchor: Some(placed),
            ..GridEdit::default()
        },
    )
    .grid;
    assert_eq!(g.anchor, placed);
    assert_eq!(g.verdict, Verdict::Static, "3 ms off is still static");
    // A bar line placed in bar 17 moves bar 1 to the same lattice's first bar.
    let later = sample(anchor_s(&base) + 16.0 * bar_len - 0.002);
    let g = fit(
        &ev,
        &GridEdit {
            anchor: Some(later),
            ..GridEdit::default()
        },
    )
    .grid;
    let want = sample(anchor_s(&base) - 0.002);
    assert!(
        g.anchor.0.abs_diff(want.0) <= 1,
        "{:?} vs {want:?}",
        g.anchor
    );
    assert!(!g.reasons.contains(&Reason::DownbeatMargin));
}

#[test]
fn a_typed_bpm_is_kept_exactly() {
    let ev = four_four(127.98);
    for typed in [128.0, 128.05, 63.99] {
        let g = fit(
            &ev,
            &GridEdit {
                bpm: Some(Bpm(typed)),
                ..GridEdit::default()
            },
        )
        .grid;
        assert_eq!(g.bpm.0, typed);
    }
}

#[test]
fn a_tap_selects_the_nearest_lattice_of_the_fitted_tempo_or_stays_as_tapped() {
    let ev = four_four(124.0);
    for (tap, want) in [
        (125.9, 124.0),
        (121.0, 124.0),
        (61.2, 62.0),
        (247.0, 248.0),
        (183.0, 186.0),
        (84.0, 124.0 * 2.0 / 3.0),
        (150.0, 150.0),
    ] {
        let g = fit(
            &ev,
            &GridEdit {
                tempo_hint: Some(Bpm(tap)),
                ..GridEdit::default()
            },
        )
        .grid;
        assert!(
            (g.bpm.0 - want).abs() < 0.005,
            "tap {tap}: {} vs {want}",
            g.bpm.0
        );
    }
}

#[test]
fn a_chosen_meter_is_used_with_its_grouping_and_the_estimate_becomes_the_runner_up() {
    // 9/8 aksak with the model's beats on the eighth-note pulse (399 per minute).
    let pulse = 60.0 / 399.0;
    let ev = evidence(&[2, 2, 2, 3], pulse, 48, 1);
    let base = fit(&ev, &GridEdit::default()).grid;
    assert_eq!(base.meter, Meter::nine_eight_aksak(), "{base:?}");
    for chosen in [
        Meter::nine_eight_long_first(),
        Meter::four_four(),
        Meter::seven_eight(),
    ] {
        let g = fit(
            &ev,
            &GridEdit {
                meter: Some(chosen.clone()),
                ..GridEdit::default()
            },
        )
        .grid;
        assert_eq!(g.meter, chosen);
        assert_eq!(g.meter_runner_up, Some(Meter::nine_eight_aksak()));
        assert!(
            !g.reasons.contains(&Reason::MeterMargin),
            "{chosen}: {:?}",
            g.reasons
        );
        if chosen.unit == BeatUnit::Eighth {
            assert!((g.bpm.0 - 399.0).abs() < 0.02, "{chosen}: {}", g.bpm.0);
        }
    }
    // Choosing the estimated meter keeps its own runner-up.
    let same = fit(
        &ev,
        &GridEdit {
            meter: Some(Meter::nine_eight_aksak()),
            ..GridEdit::default()
        },
    )
    .grid;
    assert_eq!(same.meter_runner_up, base.meter_runner_up);
    assert!(
        (anchor_s(&same) - FIRST).abs() <= 0.005,
        "{}",
        anchor_s(&same)
    );
}

#[test]
fn too_few_beats_need_a_typed_bpm_and_a_placed_bar_line() {
    let mut ev = four_four(120.0);
    ev.beats_s.truncate(4);
    ev.downbeats_s.truncate(1);
    let tags = TagHints::default();
    let ctx = Context {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        tags: &tags,
        sample_rate: SR,
    };
    assert!(refit(&ev, &ctx, &GridEdit::default()).is_none());
    let only_bpm = GridEdit {
        bpm: Some(Bpm(120.0)),
        ..GridEdit::default()
    };
    assert!(refit(&ev, &ctx, &only_bpm).is_none());
    let manual = GridEdit {
        anchor: Some(sample(FIRST + 8.0 * 2.0)),
        ..only_bpm
    };
    let solved = refit(&ev, &ctx, &manual).expect("a manual grid");
    let g = &solved.grid;
    assert_eq!(g.bpm.0, 120.0);
    assert!(
        (anchor_s(g) - FIRST).abs() <= 1.0 / f64::from(SR),
        "{}",
        anchor_s(g)
    );
    assert_eq!(g.meter, Meter::four_four());
    assert_eq!(g.confidence, Confidence::Amber);
    assert!(g.reasons.contains(&Reason::Manual));
    assert_eq!(g.verdict, Verdict::Static, "{g:?}");
    assert!(solved.lines.matched > 200, "{:?}", solved.lines.matched);
}

#[test]
fn the_residual_lane_has_one_value_per_line_with_gaps_and_the_worst_line() {
    let mut ev = four_four(120.0);
    // Beat 40 (bar 11 beat 1) lands 12 ms late; the attacks stop after bar 60.
    let late = ev
        .kick_onsets
        .frames
        .iter()
        .position(|&f| f == ((FIRST + 40.0 * 0.5) * f64::from(ANALYSIS_RATE)).round() as u32)
        .expect("beat 40");
    ev.kick_onsets.frames[late] += (0.012 * f64::from(ANALYSIS_RATE)).round() as u32;
    let solved = fit(&ev, &GridEdit::default());
    let lines = &solved.lines;
    // Bar 1 is at 0.5 s: the line at 0.0 s is the first one in the file.
    assert_eq!(lines.first_line, -1);
    assert!(
        lines.residuals_ms[0].is_nan(),
        "no attack before the first beat"
    );
    let at = |line: i64| lines.residuals_ms[usize::try_from(line - lines.first_line).unwrap()];
    assert!((at(40) - 12.0).abs() < 0.1, "{}", at(40));
    assert!(at(39).abs() < 0.1 && at(41).abs() < 0.1);
    assert_eq!(
        lines.residuals_ms.len(),
        1 + 256,
        "every line to the last beat"
    );
    assert_eq!(lines.matched, 256);
    assert_eq!(lines.attacks, 256);
    // One late attack does not move the smoothed curve's worst point far from zero, but the
    // worst line is still reported.
    assert!(lines.worst_line.is_some());
}

#[test]
fn every_matched_attack_and_the_worst_line_are_inside_the_lane() {
    let mut ev = four_four(120.0);
    // An attack 20 ms after the last beat's line, on a grid that drifts late towards the end.
    let last = *ev.kick_onsets.frames.last().unwrap();
    ev.kick_onsets
        .frames
        .push(last + (0.52 * f64::from(ANALYSIS_RATE)) as u32);
    ev.kick_onsets.rise_db.push(25.0);
    ev.kick_onsets.level_db.push(-3.0);
    let solved = fit(
        &ev,
        &GridEdit {
            bpm: Some(Bpm(120.02)),
            ..GridEdit::default()
        },
    );
    let lines = &solved.lines;
    let finite = lines.residuals_ms.iter().filter(|r| r.is_finite()).count();
    assert_eq!(finite, lines.matched as usize);
    let worst = lines.worst_line.expect("a worst line");
    let end = lines.first_line + i64::try_from(lines.residuals_ms.len()).unwrap();
    assert!(
        (lines.first_line..end).contains(&worst),
        "{worst} not in {}..{end}",
        lines.first_line
    );
}

#[test]
fn refits_are_deterministic() {
    let ev = evidence(&[2, 2, 3], 60.0 / 360.0, 40, 1);
    assert_same(
        &fit(&ev, &GridEdit::default()),
        &fit(&ev, &GridEdit::default()),
    );
    let edit = GridEdit {
        octave: 1,
        downbeat_shift: 2,
        ..GridEdit::default()
    };
    assert_same(&fit(&ev, &edit), &fit(&ev, &edit));
}
