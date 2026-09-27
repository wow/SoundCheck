//! Unit tests of the private parts of `crates/sc-analysis/src/grid/tempo.rs`.
use super::*;

#[test]
fn joint_fit_ignores_outliers_and_phase_jumps() {
    let mut beats: Vec<f64> = (0..64).map(|k| 0.3 + 0.5 * f64::from(k)).collect();
    beats[10] += 0.2;
    beats[40] -= 0.15;
    for b in &mut beats[48..] {
        *b += 0.25;
    }
    let tempo = fit_tempo(&beats).unwrap();
    assert!((tempo.period - 0.5).abs() < 1e-9, "{tempo:?}");
    assert!(tempo.coverage > 0.95, "{tempo:?}");
}
