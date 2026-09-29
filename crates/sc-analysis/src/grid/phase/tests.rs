//! Unit tests of the private parts of `crates/sc-analysis/src/grid/phase.rs`.
use super::*;

const PERIOD: f64 = 0.5;
const SPAN: (f64, f64) = (0.0, 60.0);

fn onsets_at(phase: f64, rise_db: f32) -> Vec<TimedOnset> {
    (0..120_u32)
        .map(|i| TimedOnset {
            time_s: phase + PERIOD * f64::from(i),
            rise_db,
            level_db: 0.0,
        })
        .collect()
}

/// Downbeat logits at 50 frames per second: confident every fourth line from `phase`.
fn logits_at(phase: f64) -> Vec<f32> {
    let mut logits = vec![-6.0_f32; 3_100];
    for i in 0..30_u32 {
        let t = phase + 4.0 * PERIOD * f64::from(i);
        logits[frame_index((t * 50.0).round()).expect("frame in range")] = 4.0;
    }
    logits
}

#[test]
fn the_activation_decides_the_half_beat_parity() {
    assert!(activation_prefers(
        PERIOD,
        0.0,
        0.25,
        SPAN,
        &logits_at(0.25)
    ));
    assert!(!activation_prefers(
        PERIOD,
        0.25,
        0.0,
        SPAN,
        &logits_at(0.25)
    ));
}

#[test]
fn split_weak_or_missing_activation_does_not_decide() {
    let mut split = logits_at(0.0);
    for (s, x) in split.iter_mut().zip(logits_at(0.25)) {
        *s = s.max(x);
    }
    assert!(!activation_prefers(PERIOD, 0.0, 0.25, SPAN, &split));
    // Three confident downbeats on the other lattice are not enough to move a grid.
    let mut few = vec![-6.0_f32; 3_100];
    for t in [12.25, 20.25, 40.25] {
        few[frame_index(t * 50.0).expect("frame in range")] = 4.0;
    }
    assert!(!activation_prefers(PERIOD, 0.0, 0.25, SPAN, &few));
    assert!(!activation_prefers(PERIOD, 0.0, 0.25, SPAN, &[]));
}

#[test]
fn beat_phase_follows_the_activation_past_a_louder_off_beat() {
    let mut onsets = onsets_at(0.0, 30.0);
    onsets.extend(onsets_at(0.25, 40.0));
    onsets.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    let combed = phase_from_onsets(PERIOD, 0.25, None, SPAN, &onsets);
    assert!((combed - 0.25).abs() < 1e-3, "{combed}");
    let phase = beat_phase(PERIOD, 0.25, None, SPAN, &onsets, &logits_at(0.0));
    assert!(lattice_distance(phase, PERIOD, 0.0).abs() < 1e-3, "{phase}");
    // Activation on the comb's phase: nothing moves.
    let kept = beat_phase(PERIOD, 0.25, None, SPAN, &onsets, &logits_at(0.25));
    assert!((kept - combed).abs() < 1e-12);
}
