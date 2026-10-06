//! Unit tests of `crates/sc-dsp/src/requantise.rs`, with the tolerances the file layer is held
//! to: exact shifts at 0 dB, 24-bit within 1 LSB of `round(r)` (mean error < 0.05 LSB, RMS
//! <= 0.35 LSB), 16-bit TPDF (mean < 0.05 LSB, RMS 0.4..0.6 LSB), gain RMS error against an
//! f64 reference < 1e-6 of full scale.
#![allow(clippy::float_cmp)] // exact values are intended where compared exactly
use super::*;
use crate::gain::db_to_linear;
use sc_core::{AudioSpec, testsig};

/// 2 s of 24-bit stereo noise at about -6 dBFS peak.
fn noise24() -> Vec<i32> {
    let buf = testsig::seeded_noise(AudioSpec::CD, 3, 0.5, 2.0);
    // Inside +/-0.5 of full scale, so the cast is exact after rounding.
    #[allow(clippy::cast_possible_truncation)]
    buf.data
        .iter()
        .map(|x| (f64::from(*x) * 8_388_608.0).round() as i32)
        .collect()
}

fn run_int(src_bits: u16, out_bits: u16, gain_db: f64, input: &[i32]) -> (Requantiser, Vec<i32>) {
    let mut q = Requantiser::new(SourceDepth::Int { bits: src_bits }, out_bits, gain_db, 11)
        .expect("valid settings");
    let mut out = vec![0; input.len()];
    q.push_int(input, &mut out).expect("integer source");
    (q, out)
}

/// Mean and RMS of `y - r` in output LSB, `r` being the exact scaled value.
// Sample counts stay far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn error_stats(
    input: &[i32],
    src_bits: u16,
    out: &[i32],
    out_bits: u16,
    gain_db: f64,
) -> (f64, f64, f64) {
    let g = db_to_linear(gain_db);
    let scale = 2_f64.powi(i32::from(out_bits) - 1) / 2_f64.powi(i32::from(src_bits) - 1);
    let (mut sum, mut sq, mut worst) = (0.0, 0.0, 0.0_f64);
    for (x, y) in input.iter().zip(out) {
        let r = f64::from(*x) * g * scale;
        let e = f64::from(*y) - r;
        sum += e;
        sq += e * e;
        worst = worst.max((f64::from(*y) - r.round()).abs());
    }
    let n = input.len() as f64;
    (sum / n, (sq / n).sqrt(), worst)
}

#[test]
fn zero_db_same_depth_is_bit_identical() {
    let input = noise24();
    let (q, out) = run_int(24, 24, 0.0, &input);
    assert!(q.is_exact() && !q.is_dithered());
    assert_eq!(out, input);
}

#[test]
fn zero_db_to_a_greater_depth_is_an_exact_shift() {
    let input: Vec<i32> = noise24().iter().map(|x| x >> 8).collect();
    let (q, out) = run_int(16, 24, 0.0, &input);
    assert!(q.is_exact());
    assert!(out.iter().zip(&input).all(|(y, x)| *y == x << 8));
    assert_eq!(
        run_int(16, 24, 0.0, &[-32_768, 32_767]).1,
        [-8_388_608, 8_388_352]
    );
}

#[test]
fn gain_at_24_bit_rounds_without_dither() {
    let input = noise24();
    let (q, out) = run_int(24, 24, -3.2, &input);
    assert!(!q.is_exact() && !q.is_dithered());
    // Undithered: exactly round half to even of x * g (the 24 -> 24 scale is 1).
    let k = db_to_linear(-3.2);
    for (i, (x, y)) in input.iter().zip(&out).enumerate() {
        assert_eq!(
            f64::from(*y),
            (f64::from(*x) * k).round_ties_even(),
            "sample {i}"
        );
    }
    let (mean, rms, worst) = error_stats(&input, 24, &out, 24, -3.2);
    assert!(worst <= 0.5, "worst {worst}");
    assert!(mean.abs() < 0.05, "mean {mean}");
    assert!(rms <= 0.35, "rms {rms}");
    // As a fraction of full scale the gain path is far inside 1e-6.
    assert!(rms / 8_388_608.0 < 1e-6);
    assert_eq!(q.samples_saturated(), 0);
}

#[test]
fn reduction_to_16_bit_is_tpdf_dithered() {
    let input = noise24();
    for gain_db in [0.0, -3.2, 2.1] {
        let (q, out) = run_int(24, 16, gain_db, &input);
        assert!(q.is_dithered(), "{gain_db} dB");
        let (mean, rms, _) = error_stats(&input, 24, &out, 16, gain_db);
        assert!(mean.abs() < 0.05, "{gain_db} dB: mean {mean}");
        assert!((0.4..=0.6).contains(&rms), "{gain_db} dB: rms {rms}");
    }
    let sixteen: Vec<i32> = input.iter().map(|x| x >> 8).collect();
    let (q, out) = run_int(16, 16, -3.2, &sixteen);
    assert!(q.is_dithered());
    let (_, rms, _) = error_stats(&sixteen, 16, &out, 16, -3.2);
    assert!((0.4..=0.6).contains(&rms), "16 -> 16 with gain: rms {rms}");
}

#[test]
fn float_source_rounds_at_24_and_dithers_at_16() {
    let input: Vec<f64> = noise24()
        .iter()
        .map(|x| f64::from(*x) / 8_388_608.0)
        .collect();
    let mut q = Requantiser::new(SourceDepth::Float, 24, 0.0, 1).expect("valid");
    let mut out = vec![0; input.len()];
    q.push_float(&input, &mut out).expect("float source");
    assert!(!q.is_exact() && !q.is_dithered());
    // These floats are exact 24-bit values, so 0 dB at 24 bits gives them back.
    assert!(
        out.iter()
            .zip(&input)
            .all(|(y, x)| f64::from(*y) == x * 8_388_608.0)
    );
    let mut q = Requantiser::new(SourceDepth::Float, 16, 0.0, 1).expect("valid");
    q.push_float(&input, &mut out).expect("float source");
    assert!(q.is_dithered());
}

#[test]
fn ties_round_to_even() {
    // An exact factor of 0.5 on odd integers lands every value on a tie.
    let mut q = Requantiser::with_factor(SourceDepth::Int { bits: 24 }, 24, 0.5, 0).expect("valid");
    assert!(!q.is_exact() && !q.is_dithered());
    let mut out = [0; 6];
    q.push_int(&[1, 3, 5, -1, -3, -5], &mut out)
        .expect("integer source");
    // 0.5 -> 0, 1.5 -> 2, 2.5 -> 2, -0.5 -> -0, -1.5 -> -2, -2.5 -> -2.
    assert_eq!(out, [0, 2, 2, 0, -2, -2]);
    // A factor of exactly 1 is the exact path.
    let q = Requantiser::with_factor(SourceDepth::Int { bits: 16 }, 24, 1.0, 0).expect("valid");
    assert!(q.is_exact());
    assert!(Requantiser::with_factor(SourceDepth::Float, 24, -0.5, 0).is_err());
}

#[test]
fn the_gain_factor_is_bit_identical_to_the_pure_rust_pow() {
    // Pinned bits of 10^(-3.2/20) and 10^(2.1/20) from libm's pow (musl): a platform maths
    // library that differs in the last bit would change rendered files.
    assert_eq!(db_to_linear(-3.2).to_bits(), 0x3FE6_237A_B44E_9DC6);
    assert_eq!(db_to_linear(2.1).to_bits(), 0x3FF4_6044_C445_2618);
    assert_eq!(db_to_linear(-0.5).to_bits(), 0x3FEE_35BF_27A2_98B9);
    assert_eq!(db_to_linear(0.0), 1.0);
    assert_eq!(
        db_to_linear(-6.020_599_913_279_624).to_bits(),
        0.5_f64.to_bits()
    );
}

#[test]
fn values_past_full_scale_saturate_and_are_counted() {
    let mut q = Requantiser::new(SourceDepth::Float, 24, 0.0, 0).expect("valid");
    let mut out = [0; 4];
    q.push_float(&[0.999_999_99, -1.0, 2.0, f64::NAN], &mut out)
        .expect("float source");
    assert_eq!(out, [8_388_607, -8_388_608, 8_388_607, 0]);
    assert_eq!(q.samples_saturated(), 3);
}

#[test]
fn block_size_does_not_change_the_output() {
    let input = noise24();
    let (_, whole) = run_int(24, 16, -3.2, &input);
    for block in [1, 7, 512, 65_536] {
        let mut q = Requantiser::new(SourceDepth::Int { bits: 24 }, 16, -3.2, 11).expect("valid");
        let mut got = vec![0; input.len()];
        for (i, o) in input.chunks(block).zip(got.chunks_mut(block)) {
            q.push_int(i, o).expect("integer source");
        }
        assert_eq!(got, whole, "block {block}");
    }
}

#[test]
fn invalid_settings_and_mismatched_input_are_errors() {
    assert!(Requantiser::new(SourceDepth::Float, 32, 0.0, 0).is_err());
    assert!(Requantiser::new(SourceDepth::Float, 24, f64::NAN, 0).is_err());
    assert!(Requantiser::new(SourceDepth::Int { bits: 0 }, 24, 0.0, 0).is_err());
    let mut q = Requantiser::new(SourceDepth::Float, 24, 0.0, 0).expect("valid");
    assert!(q.push_int(&[1], &mut [0]).is_err());
    let mut q = Requantiser::new(SourceDepth::Int { bits: 16 }, 24, -1.0, 0).expect("valid");
    assert!(q.push_float(&[0.1], &mut [0]).is_err());
}
