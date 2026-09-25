//! The grid solver on synthetic evidence shaped like the model's output: beats on a 20 ms frame
//! grid, downbeat activations, and kick onsets at 1 ms resolution with accents. Tolerances from
//! the analysis rules: BPM +/-0.02, bar 1 within 5 ms.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // test arithmetic on small counts

use sc_analysis::grid::{Evidence, SolveSettings, TimedOnset, line_at, solve};
use sc_core::analysis::{Confidence, Grid, Reason, Verdict};

const SR: u32 = 44_100;

/// Synthetic evidence of a steady track.
struct Synth {
    beats: Vec<f64>,
    logits: Vec<f32>,
    onsets: Vec<TimedOnset>,
}

struct Spec {
    bpm: f64,
    seconds: f64,
    first: f64,
    /// Model downbeat every this many beats.
    downbeat_every: usize,
    /// Linear change of the beat period from start to end (ppm).
    ramp_ppm: f64,
    /// Uniform onset jitter (+/- seconds).
    jitter_s: f64,
    /// Systematic offset of the model's beats against the attacks.
    model_offset_s: f64,
}

impl Spec {
    fn steady(bpm: f64) -> Self {
        Self {
            bpm,
            seconds: 90.0,
            first: 0.5,
            downbeat_every: 4,
            ramp_ppm: 0.0,
            jitter_s: 0.0,
            model_offset_s: 0.0,
        }
    }
}

/// xorshift64* for reproducible jitter.
struct Rng(u64);
impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (x >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
    }
}

fn synth(spec: &Spec) -> Synth {
    let period = 60.0 / spec.bpm;
    let n = ((spec.seconds - spec.first) / period).floor() as usize;
    let mut times = Vec::with_capacity(n);
    let mut t = spec.first;
    for i in 0..n {
        times.push(t);
        t += period * (1.0 + spec.ramp_ppm * 1e-6 * i as f64 / n as f64);
    }
    let beats = times
        .iter()
        .map(|t| ((t + spec.model_offset_s) * 50.0).round() / 50.0)
        .collect();
    let frames = (spec.seconds * 50.0).ceil() as usize + 2;
    let mut logits = vec![-6.0_f32; frames];
    for (i, t) in times.iter().enumerate() {
        let f = ((t + spec.model_offset_s) * 50.0).round() as usize;
        if f < frames {
            logits[f] = if i % spec.downbeat_every == 0 {
                4.0
            } else {
                -2.0
            };
        }
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let onsets = times
        .iter()
        .enumerate()
        .map(|(i, t)| TimedOnset {
            time_s: ((t + spec.jitter_s * rng.uniform()) * 1000.0).round() / 1000.0,
            rise_db: 30.0,
            level_db: if i % spec.downbeat_every == 0 {
                0.0
            } else {
                -3.0
            },
        })
        .collect();
    Synth {
        beats,
        logits,
        onsets,
    }
}

fn run(s: &Synth, settings: &SolveSettings) -> Grid {
    let ev = Evidence {
        beats_s: &s.beats,
        downbeat_logits: &s.logits,
        kick_onsets: &s.onsets,
        broadband_onsets: &[],
    };
    solve(&ev, settings, SR).expect("a grid")
}

fn anchor_s(g: &Grid) -> f64 {
    g.anchor.to_seconds(SR).0
}

#[test]
fn click_tempi_fit_bpm_and_bar_one_within_tolerance() {
    // (true BPM, expected BPM inside the 70-180 range, downbeat spacing in model beats)
    for (bpm, expected, downbeat_every) in [
        (60.0, 120.0, 4),
        (85.0, 85.0, 4),
        (100.0, 100.0, 4),
        (120.0, 120.0, 4),
        (127.98, 127.98, 4),
        (140.0, 140.0, 4),
        (174.0, 174.0, 4),
        (200.0, 100.0, 8),
    ] {
        let spec = Spec {
            downbeat_every,
            ..Spec::steady(bpm)
        };
        let g = run(&synth(&spec), &SolveSettings::default());
        assert!(
            (g.bpm.0 - expected).abs() <= 0.02,
            "{bpm}: got {:.4}, want {expected}",
            g.bpm.0
        );
        assert!(
            (anchor_s(&g) - 0.5).abs() <= 0.005,
            "{bpm}: bar 1 at {:.4} s",
            anchor_s(&g)
        );
        assert_eq!(g.verdict, Verdict::Static, "{bpm}: {g:?}");
        assert_eq!(g.confidence, Confidence::Green, "{bpm}: {:?}", g.reasons);
    }
}

#[test]
fn range_forces_the_octave_and_keeps_the_other_as_an_alternative() {
    let g = run(&synth(&Spec::steady(60.0)), &SolveSettings::default());
    assert!((g.bpm.0 - 120.0).abs() < 1e-9);
    assert!((g.alternatives.octave_down.unwrap().0 - 60.0).abs() < 1e-9);
    let spec = Spec {
        downbeat_every: 8,
        ..Spec::steady(200.0)
    };
    let g = run(&synth(&spec), &SolveSettings::default());
    assert!((g.bpm.0 - 100.0).abs() < 1e-9);
    assert!((g.alternatives.octave_up.unwrap().0 - 200.0).abs() < 1e-9);
}

#[test]
fn a_true_127_98_is_never_rounded_and_a_near_round_tempo_is() {
    let g = run(&synth(&Spec::steady(127.98)), &SolveSettings::default());
    assert!((g.bpm.0 - 127.98).abs() < 0.005, "{}", g.bpm.0);
    let g = run(&synth(&Spec::steady(128.0004)), &SolveSettings::default());
    assert!((g.bpm.0 - 128.0).abs() < 1e-12, "{}", g.bpm.0);
}

#[test]
fn model_beats_a_frame_late_still_anchor_on_the_attack() {
    let spec = Spec {
        model_offset_s: 0.018,
        ..Spec::steady(123.87)
    };
    let g = run(&synth(&spec), &SolveSettings::default());
    assert!(
        (anchor_s(&g) - 0.5).abs() <= 0.002,
        "bar 1 at {:.4}",
        anchor_s(&g)
    );
    assert!(g.residual_p95_ms <= 1.0, "{g:?}");
    assert_eq!(g.verdict, Verdict::Static);
}

#[test]
fn missing_and_spurious_beats_do_not_move_the_tempo() {
    let mut s = synth(&Spec::steady(132.0));
    let period = 60.0 / 132.0;
    let mut beats: Vec<f64> = s
        .beats
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 7 != 3)
        .map(|(_, t)| *t)
        .collect();
    for i in (5..150).step_by(23) {
        beats.push(((0.5 + period * (f64::from(i) + 0.37)) * 50.0).round() / 50.0);
    }
    beats.sort_by(f64::total_cmp);
    s.beats = beats;
    let g = run(&s, &SolveSettings::default());
    assert!((g.bpm.0 - 132.0).abs() <= 0.02, "{}", g.bpm.0);
    assert!((anchor_s(&g) - 0.5).abs() <= 0.005);
}

#[test]
fn a_300_ppm_tempo_ramp_is_reported_as_drifting() {
    let spec = Spec {
        seconds: 240.0,
        ramp_ppm: 300.0,
        ..Spec::steady(120.0)
    };
    let g = run(&synth(&spec), &SolveSettings::default());
    assert_eq!(g.verdict, Verdict::Drifts, "{g:?}");
    assert!(
        (f64::from(g.drift_ppm) - 300.0).abs() <= 50.0,
        "{}",
        g.drift_ppm
    );
    assert!(g.reasons.contains(&Reason::Drifts));
}

#[test]
fn fifteen_ms_of_jitter_is_static_with_a_warning() {
    let spec = Spec {
        seconds: 240.0,
        jitter_s: 0.015,
        ..Spec::steady(120.0)
    };
    let g = run(&synth(&spec), &SolveSettings::default());
    assert_eq!(g.verdict, Verdict::StaticWarn, "{g:?}");
    assert!(g.reasons.contains(&Reason::Residuals));
    assert!((g.bpm.0 - 120.0).abs() <= 0.02);
}

#[test]
fn tag_and_genre_settle_an_octave_the_range_allows_twice() {
    let s = synth(&Spec::steady(85.0));
    let tagged = SolveSettings {
        tag_bpm: Some(170.0),
        ..SolveSettings::default()
    };
    assert!((run(&s, &tagged).bpm.0 - 170.0).abs() <= 0.02);
    let s = synth(&Spec::steady(87.0));
    let dnb = SolveSettings {
        genre: Some("Drum & Bass".into()),
        ..SolveSettings::default()
    };
    assert!((run(&s, &dnb).bpm.0 - 174.0).abs() <= 0.02);
    // Without hints the kick density keeps the slower lattice (no kick between the beats).
    assert!((run(&s, &SolveSettings::default()).bpm.0 - 87.0).abs() <= 0.02);
}

#[test]
fn a_disagreeing_tag_is_flagged() {
    let s = synth(&Spec::steady(124.0));
    let tagged = SolveSettings {
        tag_bpm: Some(128.0),
        ..SolveSettings::default()
    };
    assert!(run(&s, &tagged).reasons.contains(&Reason::TagDisagrees));
}

#[test]
fn without_kick_onsets_the_broadband_attack_anchors_bar_one() {
    let s = synth(&Spec::steady(118.0));
    let ev = Evidence {
        beats_s: &s.beats,
        downbeat_logits: &s.logits,
        kick_onsets: &[],
        broadband_onsets: &s.onsets,
    };
    let g = solve(&ev, &SolveSettings::default(), SR).unwrap();
    assert!(g.reasons.contains(&Reason::NoKick));
    assert!((anchor_s(&g) - 0.5).abs() <= 0.005);
}

#[test]
fn the_earliest_significant_rise_is_the_anchor() {
    let mut s = synth(&Spec::steady(120.0));
    // A soft pre-echo 12 ms early is not significant; a strong attack 3 ms early is, and agrees
    // with the global phase, so it becomes bar 1; one 8 ms early does not agree and is ignored.
    s.onsets.push(TimedOnset {
        time_s: 0.488,
        rise_db: 10.0,
        level_db: -20.0,
    });
    s.onsets.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    assert!((anchor_s(&run(&s, &SolveSettings::default())) - 0.5).abs() <= 0.001);
    s.onsets.retain(|o| (o.time_s - 0.488).abs() > 1e-9);
    s.onsets.push(TimedOnset {
        time_s: 0.497,
        rise_db: 28.0,
        level_db: 0.0,
    });
    s.onsets.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    assert!((anchor_s(&run(&s, &SolveSettings::default())) - 0.497).abs() <= 0.001);
    s.onsets.retain(|o| (o.time_s - 0.497).abs() > 1e-9);
    s.onsets.push(TimedOnset {
        time_s: 0.492,
        rise_db: 28.0,
        level_db: 0.0,
    });
    s.onsets.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    assert!((anchor_s(&run(&s, &SolveSettings::default())) - 0.5).abs() <= 0.001);
}

#[test]
fn too_few_beats_give_no_grid() {
    let s = synth(&Spec {
        seconds: 3.0,
        ..Spec::steady(120.0)
    });
    let ev = Evidence {
        beats_s: &s.beats[..5],
        downbeat_logits: &s.logits,
        kick_onsets: &s.onsets,
        broadband_onsets: &[],
    };
    assert!(solve(&ev, &SolveSettings::default(), SR).is_none());
}

#[test]
fn user_bpm_and_anchor_are_used_exactly() {
    let s = synth(&Spec::steady(127.98));
    let pinned = SolveSettings {
        bpm_override: Some(128.05),
        anchor_override_s: Some(0.503),
        ..SolveSettings::default()
    };
    let g = run(&s, &pinned);
    assert!((g.bpm.0 - 128.05).abs() < 1e-12);
    assert!((anchor_s(&g) - 0.503).abs() < 1.0 / f64::from(SR));
    // 0.07 BPM too fast leaves the onsets about 50 ms late after 90 s: far from static.
    assert!(g.residual_max_ms > 30.0, "{g:?}");
    assert_eq!(g.verdict, Verdict::Drifts, "{g:?}");
}

#[test]
fn solving_is_deterministic_and_lines_follow_the_anchor() {
    let s = synth(&Spec::steady(126.5));
    let a = run(&s, &SolveSettings::default());
    let b = run(&s, &SolveSettings::default());
    assert_eq!(a, b);
    let spb = 60.0 * f64::from(SR) / a.bpm.0;
    let tenth = line_at(&a, 10, SR).0 as f64;
    assert!((tenth - (a.anchor.0 as f64 + 10.0 * spb)).abs() <= 1.0);
}

/// The model follows the beat for 8 s, then the off-beat for the rest of the track (seen on a
/// 141.03 BPM eurodance track): the tempo must not bend and bar 1 stays on the kicks.
#[test]
fn regression_half_beat_phase_jump_of_the_model() {
    let mut s = synth(&Spec {
        seconds: 200.0,
        ..Spec::steady(141.03)
    });
    let half = 60.0 / 141.03 / 2.0;
    for b in &mut s.beats {
        if *b > 8.0 {
            *b = ((*b + half) * 50.0).round() / 50.0;
        }
    }
    let g = run(&s, &SolveSettings::default());
    assert!((g.bpm.0 - 141.03).abs() <= 0.02, "{}", g.bpm.0);
    assert!(
        (anchor_s(&g) - 0.5).abs() <= 0.005,
        "bar 1 at {:.4}",
        anchor_s(&g)
    );
    assert_eq!(g.verdict, Verdict::Static, "{g:?}");
}
