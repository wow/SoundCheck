//! Unit tests of the private parts of `crates/sc-analysis/src/meter.rs`.
use super::*;

#[test]
fn standardise_and_correlate() {
    let s = standardise(&[1.0, 2.0, 3.0]);
    assert!((s.iter().sum::<f64>()).abs() < 1e-12);
    assert!((correlation(&[1.0, 0.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0]) - 1.0).abs() < 1e-12);
    assert_eq!(standardise(&[2.0; 4]), vec![0.0; 4]);
}

#[test]
fn quarter_level_candidates_are_only_the_simple_meters() {
    assert_eq!(candidates(Level::Beat).len(), 2);
    let triple = candidates(Level::Triple);
    assert_eq!(triple[0].groups, vec![3, 3, 3, 3]);
    assert_eq!(triple.len(), 3);
    let duple = candidates(Level::Duple);
    assert_eq!(duple.len(), 8);
    assert_eq!(duple[0].groups, vec![2, 2, 2, 2]);
    assert_eq!(duple[3].groups, vec![2, 2, 2, 3]);
    assert_eq!(candidates(Level::Fast).len(), 9);
}
