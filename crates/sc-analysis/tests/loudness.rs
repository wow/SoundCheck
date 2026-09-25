//! Loudness on synthetic signals: the EBU Tech 3341 tone cases the test set also contains, the
//! dual-mono rule, silence, block-size invariance, the DJ statistics on a two-level signal, and
//! true peak against sample peak. Tolerances: +/-0.05 LU on tones (rules), +/-0.1 dB peaks.

#![allow(clippy::float_cmp)] // exact values are intended in these tests

use approx::assert_abs_diff_eq;
use sc_analysis::loudness::{LoudnessMeter, measure};
use sc_core::units::MIN_DBFS;
use sc_core::{AudioBuffer, AudioSpec, SampleIndex, testsig};

/// A 1 kHz tone at `dbfs` peak level.
fn tone(spec: AudioSpec, dbfs: f64, seconds: f64) -> AudioBuffer {
    // Test fixture level; the value is small and positive.
    #[allow(clippy::cast_possible_truncation)]
    let amplitude = 10f64.powf(dbfs / 20.0) as f32;
    testsig::sine(spec, 1000.0, amplitude, seconds)
}

#[test]
fn stereo_tone_at_minus_23_dbfs_reads_minus_23_lufs() {
    let report = measure(&tone(AudioSpec::CD, -23.0, 20.0)).unwrap();
    assert_abs_diff_eq!(report.integrated.unwrap().0, -23.0, epsilon = 0.05);
    assert_abs_diff_eq!(report.momentary_max.unwrap().0, -23.0, epsilon = 0.05);
    assert_abs_diff_eq!(report.short_term_max.unwrap().0, -23.0, epsilon = 0.05);
    assert_abs_diff_eq!(report.short_term_p95.unwrap().0, -23.0, epsilon = 0.05);
    assert!(
        report.lra.unwrap().0 <= 0.2,
        "steady tone has no range: {:?}",
        report.lra
    );
    assert_abs_diff_eq!(report.sample_peak.0, -23.0, epsilon = 0.05);
    assert_abs_diff_eq!(report.true_peak.0, -23.0, epsilon = 0.1);
    assert!(report.true_peak.0 >= report.sample_peak.0);
    assert_abs_diff_eq!(report.plr.unwrap().0, 0.0, epsilon = 0.15);
    assert!(!report.dual_mono);
    assert_eq!(report.timeline.hop_ms, 100);
    assert_eq!(report.timeline.short_term.len(), 200);
    assert_abs_diff_eq!(
        report.timeline.short_term[100].unwrap(),
        -23.0,
        epsilon = 0.05
    );
}

#[test]
fn mono_tone_is_measured_as_dual_mono() {
    let report = measure(&tone(AudioSpec::new(48_000, 1), -23.0, 20.0)).unwrap();
    assert!(report.dual_mono);
    // One channel at -23 dBFS would read -26.0 LUFS on its own; dual mono doubles the energy
    // (+3.01 LU), so the mono file reads the same -23.0 as the stereo tone.
    assert_abs_diff_eq!(report.integrated.unwrap().0, -23.0, epsilon = 0.05);
    assert_abs_diff_eq!(report.sample_peak.0, -23.0, epsilon = 0.05);
}

#[test]
fn silence_reports_none_everywhere() {
    let report = measure(&AudioBuffer::silence(AudioSpec::CD, 20 * 44_100)).unwrap();
    assert_eq!(report.integrated, None);
    assert_eq!(report.momentary_max, None);
    assert_eq!(report.short_term_max, None);
    assert_eq!(report.short_term_p95, None);
    assert_eq!(report.short_term_top30, None);
    assert_eq!(report.lra, None);
    assert_eq!(report.plr, None);
    assert_eq!(report.sample_peak.0, MIN_DBFS);
    assert_eq!(report.true_peak.0, MIN_DBFS);
    assert_eq!(report.timeline.short_term.len(), 200);
    assert!(report.timeline.short_term.iter().all(Option::is_none));
}

#[test]
fn block_size_does_not_change_a_single_number() {
    let buf = testsig::seeded_noise(AudioSpec::CD, 7, 0.25, 12.3);
    let whole = measure(&buf).unwrap();
    for block_frames in [1_usize, 7, 4_096, 44_100] {
        let mut meter = LoudnessMeter::new(buf.spec).unwrap();
        for block in buf.data.chunks(block_frames * 2) {
            meter.push(block);
        }
        assert_eq!(meter.finish(), whole, "block of {block_frames} frames");
    }
}

#[test]
fn quiet_intro_then_loud_body_separates_the_dj_statistics() {
    let spec = AudioSpec::CD;
    let mut data = tone(spec, -33.0, 10.0).data;
    data.extend_from_slice(&tone(spec, -23.0, 33.0).data);
    let report = measure(&AudioBuffer::new(spec, data)).unwrap();
    // Both levels survive the -10 LU relative gate: power mean of 10 s at -33 and 33 s at -23.
    let expected_i = 10.0 * ((10.0 * 10f64.powf(-3.3) + 33.0 * 10f64.powf(-2.3)) / 43.0).log10();
    assert_abs_diff_eq!(report.integrated.unwrap().0, expected_i, epsilon = 0.2);
    assert_abs_diff_eq!(report.short_term_p95.unwrap().0, -23.0, epsilon = 0.1);
    assert_abs_diff_eq!(report.short_term_top30.unwrap().0, -23.0, epsilon = 0.1);
    let lra = report.lra.unwrap().0;
    assert!((8.5..=11.5).contains(&lra), "LRA {lra}");
    assert_eq!(report.timeline.short_term.len(), 430);
}

#[test]
fn true_peak_is_never_below_sample_peak() {
    let mut buf = testsig::impulse(AudioSpec::CD, SampleIndex(1_000), 1.0);
    // A second impulse on the other channel, half scale.
    let idx = 2 * 2_000 + 1;
    buf.data[idx] = 0.5;
    let report = measure(&buf).unwrap();
    assert_abs_diff_eq!(report.sample_peak.0, 0.0, epsilon = 1e-6);
    assert!(report.true_peak.0 >= report.sample_peak.0 - 1e-9);
    assert!(report.true_peak.0 < 1.0, "true peak {}", report.true_peak);
}

#[test]
fn a_short_track_has_no_top30() {
    let report = measure(&tone(AudioSpec::CD, -20.0, 20.0)).unwrap();
    assert_eq!(report.short_term_top30, None);
    let report = measure(&tone(AudioSpec::CD, -20.0, 40.0)).unwrap();
    assert_abs_diff_eq!(report.short_term_top30.unwrap().0, -20.0, epsilon = 0.05);
}
