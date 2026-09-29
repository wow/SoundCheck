//! The start fit on synthetic evidence shaped like the cached model output (beats and downbeats
//! on the model's 20 ms frames, downbeat activations, kick onsets at 1 ms): a track whose tempo
//! changes gets the start's tempo and bar 1 on its first beat and still drifts; a change inside
//! the 128-beat window shrinks it, a breakdown does not; an intro without drums moves it, a stray
//! hit in the intro does not; a steady track gets the same grid either way. Tolerances: BPM
//! +/-0.005 (two decimals exact), bar 1 within 5 ms. The tempo-change tracks open on round tempi:
//! a 32-beat window leaves the model's 20 ms beats a spread of about 0.025 BPM, within which the
//! round BPM is taken.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // test arithmetic on small counts

use sc_analysis::grid::{Evidence, SolveSettings, Solved, TimedOnset, line_share, solve_detailed};
use sc_analysis::refit::start::{START_BEATS, START_BEATS_WIDE};
use sc_analysis::refit::{Context, grid_attacks, refit, start_window_s, timed};
use sc_core::Bpm;
use sc_core::analysis::{Grid, GridEdit, GridEvidence, GridFit, OnsetList, TagHints, Verdict};

const SR: u32 = 44_100;
const ANALYSIS_RATE: u32 = 22_050;
const FIRST: f64 = 0.5;

/// Evidence of a 4/4 track from `FIRST` to `end_s` whose tempo at time `t` is `bpm_at(t)`, with
/// a kick on every beat; beats on the model's 20 ms frames, downbeats on every fourth beat.
fn track(bpm_at: impl Fn(f64) -> f64, end_s: f64) -> GridEvidence {
    let mut times = Vec::new();
    let mut t = FIRST;
    while t < end_s {
        times.push(t);
        t += 60.0 / bpm_at(t);
    }
    let onsets = |rise_db: f32| OnsetList {
        sample_rate: ANALYSIS_RATE,
        frames: times
            .iter()
            .map(|t| ((t * 1000.0).round() / 1000.0 * f64::from(ANALYSIS_RATE)).round() as u32)
            .collect(),
        rise_db: vec![rise_db; times.len()],
        level_db: (0..times.len())
            .map(|i| if i % 4 == 0 { 0.0 } else { -3.0 })
            .collect(),
    };
    let mut logits = vec![-6.0_f32; (end_s * 50.0).ceil() as usize + 2];
    let mut beats_s = Vec::with_capacity(times.len());
    let mut downbeats_s = Vec::new();
    for (i, t) in times.iter().enumerate() {
        let frame = (t * 50.0).round();
        let beat = (frame / 50.0) as f32;
        beats_s.push(beat);
        if i % 4 == 0 {
            downbeats_s.push(beat);
            logits[frame as usize] = 4.0;
        }
    }
    GridEvidence {
        beats_s,
        downbeats_s,
        downbeat_logits_50fps: logits,
        kick_onsets: onsets(30.0),
        broadband_onsets: onsets(20.0),
    }
}

/// `bpm_a` until `change_s`, then `bpm_b` until `end_s`.
fn two_tempo(bpm_a: f64, change_s: f64, bpm_b: f64, end_s: f64) -> GridEvidence {
    track(|t| if t < change_s { bpm_a } else { bpm_b }, end_s)
}

/// 116.0 BPM for a minute, then 117.2 to three minutes.
fn changing() -> GridEvidence {
    two_tempo(116.0, 60.0, 117.2, 180.0)
}

/// Keeps only the attacks (kick and broadband) for which `keep(index, time_s)` holds.
fn keep_attacks(ev: &mut GridEvidence, keep: impl Fn(usize, f64) -> bool) {
    for list in [&mut ev.kick_onsets, &mut ev.broadband_onsets] {
        let kept: Vec<usize> = (0..list.frames.len())
            .filter(|&i| keep(i, f64::from(list.frames[i]) / f64::from(ANALYSIS_RATE)))
            .collect();
        list.frames = kept.iter().map(|&i| list.frames[i]).collect();
        list.rise_db = kept.iter().map(|&i| list.rise_db[i]).collect();
        list.level_db = kept.iter().map(|&i| list.level_db[i]).collect();
    }
}

fn fit(ev: &GridEvidence, fit: GridFit) -> Grid {
    let tags = TagHints::default();
    let ctx = Context {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        tags: &tags,
        sample_rate: SR,
    };
    let edit = GridEdit {
        fit,
        ..GridEdit::default()
    };
    refit(ev, &ctx, &edit).expect("a grid").grid
}

fn anchor_s(g: &Grid) -> f64 {
    g.anchor.to_seconds(SR).0
}

#[test]
fn the_start_fit_takes_the_opening_tempo_and_bar_one_and_still_drifts() {
    let ev = changing();
    let start = fit(&ev, GridFit::Start);
    assert!((start.bpm.0 - 116.0).abs() <= 0.01, "start {}", start.bpm.0);
    assert!(
        (anchor_s(&start) - FIRST).abs() <= 0.005,
        "bar 1 at {:.4}",
        anchor_s(&start)
    );
    assert_eq!(start.verdict, Verdict::Drifts, "{start:?}");
    let whole = fit(&ev, GridFit::Whole);
    assert!(
        (whole.bpm.0 - 116.0).abs() > 0.01,
        "the whole-track fit takes the middle: {}",
        whole.bpm.0
    );
    assert_eq!(whole.verdict, Verdict::Drifts);
    assert_eq!(start, fit(&ev, GridFit::Start), "deterministic");
}

#[test]
fn the_start_fit_holds_more_of_the_opening_kicks() {
    let ev = changing();
    let window = start_window_s(&ev).expect("a window");
    // The change is after 116 beats: the whole 128-beat window (of the whole-track tempo, 116.0
    // to 117.2: 65.5 to 66.2 s) still holds one tempo well enough.
    assert_eq!(window.beats, START_BEATS);
    assert!((window.from_s - FIRST).abs() <= 0.01, "{window:?}");
    assert!(
        (window.to_s - window.from_s - 128.0 * 60.0 / 116.6).abs() < 0.5,
        "{window:?}"
    );
    let attacks = grid_attacks(&ev);
    let (from, to) = (window.from_s, window.to_s);
    let start = line_share(&fit(&ev, GridFit::Start), &attacks, SR, from, to);
    let whole = line_share(&fit(&ev, GridFit::Whole), &attacks, SR, from, to);
    assert!(start > 0.85 && start > whole + 0.2, "{start} vs {whole}");
}

#[test]
fn a_tempo_change_inside_the_window_shrinks_it_to_the_opening_tempo() {
    for (a, b) in [(116.0, 117.2), (110.0, 124.0)] {
        // The change a quarter and half way into a 128-beat window of the first tempo.
        for (quarter, beats) in [(1.0, 32), (2.0, 64)] {
            let change_s = FIRST + 32.0 * quarter * 60.0 / a;
            let ev = two_tempo(a, change_s, b, 180.0);
            let window = start_window_s(&ev).expect("a window");
            assert_eq!(
                window.beats, beats,
                "{a}->{b} at {change_s:.1} s: {window:?}"
            );
            let start = fit(&ev, GridFit::Start);
            let whole = fit(&ev, GridFit::Whole);
            assert!(
                (start.bpm.0 - a).abs() <= 0.005,
                "{a}->{b} at {change_s:.1} s: {}",
                start.bpm.0
            );
            assert!((anchor_s(&start) - FIRST).abs() <= 0.005, "{start:?}");
            let attacks = grid_attacks(&ev);
            let share = |g: &Grid| line_share(g, &attacks, SR, window.from_s, window.to_s);
            assert!(
                share(&start) > share(&whole),
                "{a}->{b}: {} vs {}",
                share(&start),
                share(&whole)
            );
        }
    }
}

#[test]
fn a_steady_track_gets_the_same_tempo_from_either_fit() {
    for bpm in [96.5, 124.0, 127.98, 174.0] {
        let ev = two_tempo(bpm, f64::INFINITY, bpm, 240.0);
        assert_eq!(start_window_s(&ev).map(|w| w.beats), Some(START_BEATS));
        let (start, whole) = (fit(&ev, GridFit::Start), fit(&ev, GridFit::Whole));
        assert!(
            (start.bpm.0 - whole.bpm.0).abs() <= 0.005,
            "{bpm}: {} vs {}",
            start.bpm.0,
            whole.bpm.0
        );
        assert!(
            (anchor_s(&start) - anchor_s(&whole)).abs() <= 0.005,
            "{bpm}"
        );
        assert_eq!(start.meter, whole.meter, "{bpm}");
    }
}

#[test]
fn a_creeping_tempo_keeps_the_longest_window() {
    // 116.0 creeping to 116.6 over six minutes, as a live drummer's does: 128 beats hold it.
    let ev = track(|t| 116.0 + 0.6 * t / 360.0, 360.0);
    let window = start_window_s(&ev).expect("a window");
    assert_eq!(window.beats, START_BEATS, "{window:?}");
    let start = fit(&ev, GridFit::Start);
    // The first 66 s average 116.055.
    assert!((start.bpm.0 - 116.055).abs() <= 0.005, "{}", start.bpm.0);
}

#[test]
fn an_intro_without_drums_starts_the_window_at_the_first_attack() {
    let mut ev = changing();
    keep_attacks(&mut ev, |_, t| t >= 120.0);
    let window = start_window_s(&ev).expect("a window");
    assert!(
        window.from_s >= 119.99 && window.from_s < 120.0 + 60.0 / 117.2,
        "{window:?}"
    );
    let start = fit(&ev, GridFit::Start);
    assert!((start.bpm.0 - 117.2).abs() <= 0.005, "{}", start.bpm.0);
    // Bar 1 is still the first bar line from the first beat of the track.
    let bar = 4.0 * 60.0 / start.bpm.0;
    assert!(
        anchor_s(&start) >= FIRST - 0.5 * 60.0 / start.bpm.0 && anchor_s(&start) < FIRST + bar,
        "bar 1 at {}",
        anchor_s(&start)
    );
}

#[test]
fn a_breakdown_in_the_opening_keeps_the_longest_window_and_the_exact_tempo() {
    for bpm in [127.98, 124.03, 116.02, 96.53] {
        for (from, to) in [(48, 110), (40, 70), (64, 96)] {
            let mut ev = two_tempo(bpm, f64::INFINITY, bpm, 240.0);
            // No attacks on beats `from` to `to` (the model still finds the beats).
            let beat = |i: i32| FIRST + f64::from(i) * 60.0 / bpm - 0.01;
            keep_attacks(&mut ev, |_, t| t < beat(from) || t >= beat(to));
            let window = start_window_s(&ev).expect("a window");
            assert_eq!(window.beats, START_BEATS, "{bpm} {from}-{to}: {window:?}");
            assert!(window.precision > 0.99, "{window:?}");
            let start = fit(&ev, GridFit::Start);
            assert!(
                (start.bpm.0 - bpm).abs() <= 0.005,
                "{bpm} {from}-{to}: {}",
                start.bpm.0
            );
            assert!((anchor_s(&start) - FIRST).abs() <= 0.005, "{start:?}");
        }
    }
}

#[test]
fn a_stray_hit_in_the_intro_does_not_start_the_window() {
    let mut ev = changing();
    keep_attacks(&mut ev, |_, t| t >= 75.0);
    for list in [&mut ev.kick_onsets, &mut ev.broadband_onsets] {
        list.frames.insert(0, ANALYSIS_RATE);
        list.rise_db.insert(0, 30.0);
        list.level_db.insert(0, 0.0);
    }
    let window = start_window_s(&ev).expect("a window");
    assert!(
        window.from_s >= 74.99 && window.from_s < 75.0 + 60.0 / 117.2,
        "{window:?}"
    );
    let start = fit(&ev, GridFit::Start);
    assert!((start.bpm.0 - 117.2).abs() <= 0.005, "{}", start.bpm.0);
}

#[test]
fn too_few_attacks_at_the_start_means_no_start_fit() {
    let mut ev = changing();
    // About 29 attacks, all in the last 15 s.
    keep_attacks(&mut ev, |_, t| t >= 165.0);
    assert_eq!(start_window_s(&ev), None);
    assert_eq!(fit(&ev, GridFit::Start), fit(&ev, GridFit::Whole));
}

#[test]
fn a_sparse_start_widens_the_window() {
    let mut ev = two_tempo(116.0, f64::INFINITY, 116.0, 180.0);
    // An attack on every fifth beat: 26 in 128 beats, 38 in 192.
    keep_attacks(&mut ev, |i, _| i % 5 == 0);
    let window = start_window_s(&ev).expect("a window");
    assert_eq!(window.beats, START_BEATS_WIDE, "{window:?}");
    // Across a tempo change the widened window blends two tempi, and a shorter one wins.
    let mut changing = changing();
    keep_attacks(&mut changing, |i, _| i % 5 == 0);
    let window = start_window_s(&changing).expect("a window");
    assert!(window.beats < START_BEATS, "{window:?}");
    let start = fit(&changing, GridFit::Start);
    assert!((start.bpm.0 - 116.0).abs() <= 0.005, "{}", start.bpm.0);
}

/// The solver on the cached evidence, fitted within `fit_window_s`.
fn solve(ev: &GridEvidence, fit_window_s: Option<(f64, f64)>) -> Solved {
    let beats: Vec<f64> = ev.beats_s.iter().map(|&b| f64::from(b)).collect();
    let downbeats: Vec<f64> = ev.downbeats_s.iter().map(|&b| f64::from(b)).collect();
    let kick = timed(&ev.kick_onsets);
    let broadband = timed(&ev.broadband_onsets);
    let evidence = Evidence {
        beats_s: &beats,
        downbeats_s: &downbeats,
        downbeat_logits: &ev.downbeat_logits_50fps,
        kick_onsets: &kick,
        broadband_onsets: &broadband,
    };
    let settings = SolveSettings {
        fit_window_s,
        ..SolveSettings::default()
    };
    solve_detailed(&evidence, &settings, SR).expect("a grid")
}

#[test]
fn a_start_with_too_few_beats_falls_back_to_the_whole_track() {
    let ev = changing();
    let whole = solve(&ev, None);
    // Four beats: fewer than the fit needs.
    let short = solve(&ev, Some((0.0, FIRST + 3.5 * 60.0 / 116.0)));
    assert_eq!(short.grid, whole.grid);
    let bits = |s: &Solved| {
        s.lines
            .residuals_ms
            .iter()
            .map(|r| r.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&short), bits(&whole), "the same residual lane");
    // Before the first beat, past the last one, and around every beat.
    assert_eq!(solve(&ev, Some((0.0, 0.1))).grid, whole.grid);
    assert_eq!(solve(&ev, Some((1e9, 2e9))).grid, whole.grid);
    assert_eq!(solve(&ev, Some((0.0, 1e9))).grid, whole.grid);
    // A real window changes the tempo but judges the whole track: the residual lane still runs
    // to its end.
    let start = solve(&ev, Some((0.0, 60.0)));
    assert!(
        (start.grid.bpm.0 - 116.0).abs() <= 0.01,
        "{}",
        start.grid.bpm.0
    );
    let lane_end_s = anchor_s(&start.grid)
        + (start.lines.first_line as f64 + start.lines.residuals_ms.len() as f64) * 60.0
            / start.grid.bpm.0;
    assert!(lane_end_s > 170.0, "{lane_end_s}");
}

fn onsets_at(times: &[f64]) -> Vec<TimedOnset> {
    times
        .iter()
        .map(|&time_s| TimedOnset {
            time_s,
            rise_db: 20.0,
            level_db: -6.0,
        })
        .collect()
}

#[test]
fn line_share_counts_the_lines_holding_an_attack_within_20_ms() {
    // 120 BPM from 1.0 s: lines at 1.0, 1.5, ..., 5.0 within 1..=5 s (9 lines).
    let grid = Grid {
        anchor: sc_core::Seconds(1.0).to_sample_index(SR),
        bpm: Bpm(120.0),
        ..fit(&changing(), GridFit::Whole)
    };
    let all: Vec<f64> = (0..9).map(|i| 1.0 + 0.5 * f64::from(i)).collect();
    assert!((line_share(&grid, &onsets_at(&all), SR, 1.0, 5.0) - 1.0).abs() < 1e-12);
    // Three lines: one attack 19 ms late, one 21 ms early (missed), two on the same line
    // (counted once), one outside the range.
    let some = [1.019, 1.479, 2.0, 2.005, 6.0];
    let share = line_share(&grid, &onsets_at(&some), SR, 1.0, 5.0);
    assert!((share - 2.0 / 9.0).abs() < 1e-12, "{share}");
    assert_eq!(
        line_share(&grid, &[], SR, 1.0, 5.0).to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        line_share(&grid, &onsets_at(&all), SR, 5.1, 5.2).to_bits(),
        0.0_f64.to_bits()
    );
    // Order does not matter.
    let mut reversed = some;
    reversed.reverse();
    assert_eq!(
        line_share(&grid, &onsets_at(&reversed), SR, 1.0, 5.0).to_bits(),
        share.to_bits()
    );
}
