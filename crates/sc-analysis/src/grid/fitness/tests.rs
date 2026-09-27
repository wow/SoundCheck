//! Unit tests of the private parts of `crates/sc-analysis/src/grid/fitness.rs`.
use super::*;

#[test]
fn inverse_of_a_symmetric_matrix() {
    let m = [[4.0, 1.0, 2.0], [1.0, 3.0, 0.5], [2.0, 0.5, 5.0]];
    let inv = invert3(m).unwrap();
    for (i, row) in m.iter().enumerate() {
        for (j, _) in inv.iter().enumerate() {
            let product: f64 = row
                .iter()
                .zip(&inv)
                .map(|(a, inv_row)| a * inv_row[j])
                .sum();
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!((product - expected).abs() < 1e-12);
        }
    }
}

#[test]
fn drift_recovers_a_linear_tempo_change() {
    // Period grows linearly by 500 ppm over 400 beats.
    let n = 400;
    let mut t = 1.0;
    let pairs: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let pair = (f64::from(k), t);
            t += 0.5 + 0.5 * 500e-6 * f64::from(k) / f64::from(n - 1);
            pair
        })
        .collect();
    let (ppm, sigma) = drift(&pairs);
    assert!((ppm - 500.0).abs() < 5.0, "{ppm} +/- {sigma}");
}
