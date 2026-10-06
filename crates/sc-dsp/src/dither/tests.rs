//! Unit tests of `crates/sc-dsp/src/dither.rs`: range, moments, triangular shape, determinism.
use super::*;

const N: usize = 1_000_000;

fn values(seed: u64, n: usize) -> Vec<f64> {
    let mut d = Tpdf::new(seed);
    let mut v = vec![0.0; n];
    d.fill(&mut v);
    v
}

#[test]
fn values_lie_in_the_open_triangle_support() {
    assert!(values(1, N).iter().all(|x| (-1.0..1.0).contains(x)));
}

// Sample counts stay far below 2^52.
#[allow(clippy::cast_precision_loss)]
#[test]
fn mean_is_zero_and_variance_one_sixth() {
    let v = values(42, N);
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    // Standard error of the mean: sqrt(1/6 / 1e6) = 4.1e-4; 5 sigma.
    assert!(mean.abs() < 2.1e-3, "mean {mean}");
    // Variance of a triangular (-1, 1) density is 1/6.
    assert!((var - 1.0 / 6.0).abs() < 2e-3, "variance {var}");
}

// Sample counts and bin indices stay far below 2^52; bins are small non-negative integers.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#[test]
fn histogram_is_triangular() {
    const BINS: usize = 20;
    let mut counts = [0_usize; BINS];
    for x in values(7, N) {
        let bin = (f64::midpoint(x, 1.0) * BINS as f64) as usize;
        counts[bin.min(BINS - 1)] += 1;
    }
    for (i, c) in counts.iter().enumerate() {
        // Probability of bin [a, b) under the triangle 1 - |x| on (-1, 1).
        let w = 2.0 / BINS as f64;
        let (a, b) = (-1.0 + w * i as f64, -1.0 + w * (i + 1) as f64);
        let cdf = |x: f64| {
            if x < 0.0 {
                (1.0 + x) * (1.0 + x) / 2.0
            } else {
                1.0 - (1.0 - x) * (1.0 - x) / 2.0
            }
        };
        let expected = (cdf(b) - cdf(a)) * N as f64;
        let rel = (*c as f64 - expected).abs() / expected;
        // The smallest bins expect 2,500 counts (standard error 2 %); 10 % allows 5 sigma.
        assert!(rel < 0.10, "bin {i}: {c} counts, expected {expected:.0}");
    }
}

#[test]
fn same_seed_gives_the_same_sequence_and_different_seeds_differ() {
    assert_eq!(values(99, 4096), values(99, 4096));
    assert_ne!(values(99, 4096), values(100, 4096));
    // Zero is a valid seed, not a stuck generator.
    let zero = values(0, 64);
    assert!(zero.windows(2).any(|w| w[0].to_bits() != w[1].to_bits()));
}

#[test]
fn fill_matches_next_lsb_one_by_one_at_any_block_size() {
    let whole = values(5, 10_000);
    for block in [1, 7, 512, 4096] {
        let mut d = Tpdf::new(5);
        let mut got = Vec::new();
        let mut buf = vec![0.0; block];
        while got.len() < whole.len() {
            d.fill(&mut buf);
            got.extend_from_slice(&buf);
        }
        got.truncate(whole.len());
        assert_eq!(got, whole, "block {block}");
    }
    let mut d = Tpdf::new(5);
    assert_eq!(d.next_lsb().to_bits(), whole[0].to_bits());
}
