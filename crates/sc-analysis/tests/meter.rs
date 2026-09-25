//! Meter estimation and bar 1 on synthetic accented pulse trains: every pulse has an attack,
//! group starts are louder and carry a kick, bar starts are loudest and carry the model's
//! downbeat activation. The model's beats are at the quarter note for 4/4 and 3/4 and at the
//! eighth-note pulse for the x/8 meters (what beat trackers do on aksak). Meter and grouping must
//! be exact and bar 1 within 5 ms.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // test arithmetic on small counts

use sc_analysis::grid::{Evidence, SolveSettings, TimedOnset, fit_beats, solve};
use sc_analysis::meter::{MeterEstimate, estimate};
use sc_core::analysis::{BeatUnit, Grid, Meter};

const SR: u32 = 44_100;
const FIRST: f64 = 0.5;

struct Track {
    beats: Vec<f64>,
    downbeats: Vec<f64>,
    logits: Vec<f32>,
    kick: Vec<TimedOnset>,
    broadband: Vec<TimedOnset>,
}

/// `bars` bars of `groups` (pulses per group) at `pulse` seconds per pulse; the model's beats
/// fall every `model_every` pulses.
fn track(groups: &[u8], pulse: f64, bars: usize, model_every: usize) -> Track {
    let bar: usize = groups.iter().map(|&g| usize::from(g)).sum();
    let mut starts = vec![false; bar];
    let mut pos = 0;
    for &g in groups {
        starts[pos] = true;
        pos += usize::from(g);
    }
    let seconds = FIRST + pulse * (bar * bars) as f64 + 1.0;
    let mut logits = vec![-6.0_f32; (seconds * 50.0).ceil() as usize + 2];
    let (mut beats, mut downbeats, mut kick, mut broadband) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
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
        let onset = TimedOnset {
            time_s: (t * 1000.0).round() / 1000.0,
            rise_db: 25.0,
            level_db: level,
        };
        broadband.push(onset);
        if starts[q] {
            kick.push(onset);
        }
        if j % model_every == 0 {
            beats.push((t * 50.0).round() / 50.0);
            if q == 0 {
                downbeats.push((t * 50.0).round() / 50.0);
            }
            let f = (t * 50.0).round() as usize;
            logits[f] = if q == 0 { 3.0 } else { -3.0 };
        }
    }
    Track {
        beats,
        downbeats,
        logits,
        kick,
        broadband,
    }
}

fn analyse(t: &Track, hint: &str) -> (MeterEstimate, Grid) {
    let fit = fit_beats(&t.beats).expect("beats fit");
    let est = estimate(
        fit.period,
        fit.phase,
        fit.span,
        &t.kick,
        &t.broadband,
        &t.logits,
        hint,
    );
    let settings = SolveSettings {
        meter: est.meter.clone(),
        meter_margin: est.margin,
        fixed_period: (est.meter.unit == BeatUnit::Eighth).then_some(est.unit_period),
        ..SolveSettings::default()
    };
    let ev = Evidence {
        beats_s: &t.beats,
        downbeats_s: &t.downbeats,
        downbeat_logits: &t.logits,
        kick_onsets: &t.kick,
        broadband_onsets: &t.broadband,
    };
    let grid = solve(&ev, &settings, SR).expect("grid");
    (est, grid)
}

fn assert_meter(groups: &[u8], pulse: f64, model_every: usize, expected: &Meter, hint: &str) {
    let t = track(groups, pulse, 48, model_every);
    let (est, grid) = analyse(&t, hint);
    assert_eq!(&est.meter, expected, "{groups:?}: {est:?}");
    assert_eq!(&grid.meter, expected);
    let bar1 = grid.anchor.to_seconds(SR).0;
    assert!(
        (bar1 - FIRST).abs() <= 0.005,
        "{groups:?}: bar 1 at {bar1:.4} s"
    );
    let expected_bpm = if expected.unit == BeatUnit::Quarter {
        60.0 / (2.0 * pulse)
    } else {
        60.0 / pulse
    };
    assert!(
        (grid.bpm.0 - expected_bpm).abs() <= 0.02,
        "{groups:?}: {} vs {expected_bpm}",
        grid.bpm.0
    );
}

#[test]
fn four_four_and_three_four_at_the_quarter_note() {
    assert_meter(&[2, 2, 2, 2], 0.25, 2, &Meter::four_four(), "");
    assert_meter(&[2, 2, 2], 0.25, 2, &Meter::three_four(), "");
}

#[test]
fn six_eight_whether_the_model_counts_eighths_or_dotted_quarters() {
    assert_meter(&[3, 3], 0.2, 1, &Meter::six_eight(), "");
    assert_meter(&[3, 3], 0.2, 3, &Meter::six_eight(), "");
}

#[test]
fn aksak_nine_eight_in_both_groupings() {
    assert_meter(
        &[2, 2, 2, 3],
        0.22,
        1,
        &Meter::nine_eight_aksak(),
        "Türk Halk",
    );
    assert_meter(
        &[3, 2, 2, 2],
        0.22,
        1,
        &Meter::nine_eight_long_first(),
        "Balkan",
    );
}

#[test]
fn five_seven_and_ten_eight() {
    assert_meter(&[2, 3], 0.2, 1, &Meter::five_eight(), "");
    assert_meter(&[2, 2, 3], 0.2, 1, &Meter::seven_eight(), "");
    assert_meter(&[3, 2, 2, 3], 0.2, 1, &Meter::ten_eight(), "");
}

#[test]
fn a_four_four_track_never_has_nine_eight_as_runner_up() {
    let t = track(&[2, 2, 2, 2], 0.25, 48, 2);
    let (est, _) = analyse(&t, "");
    assert_ne!(est.runner_up, Some(Meter::nine_eight_aksak()));
    assert_ne!(est.runner_up, Some(Meter::nine_eight_long_first()));
    assert!(est.margin >= 0.8, "{est:?}");
}
