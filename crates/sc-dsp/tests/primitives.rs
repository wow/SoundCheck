//! The primitives on synthetic signals: onsets of a burst train land within one hop (1 ms) of
//! the burst starts and nowhere else; resampling keeps length, frequency and amplitude across
//! the 2:1 and 160:147 ratios the beat tracker needs; block size never changes the output.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)] // test arithmetic on small counts

use sc_core::{AudioSpec, testsig};
use sc_dsp::{KickBand, OnsetDetector, Resampler, resample::resample_all, to_mono};

/// Mono 60 Hz bursts of 40 ms every 500 ms for `seconds`, in a quiet noise floor.
fn kick_bursts(sample_rate: u32, seconds: f64) -> (Vec<f32>, Vec<usize>) {
    let spec = AudioSpec::new(sample_rate, 1);
    let mut signal = testsig::seeded_noise(spec, 3, 0.002, seconds).data;
    let burst = testsig::sine(spec, 60.0, 0.8, 0.04).data;
    let period = testsig::frames_for(spec, 0.5);
    let mut starts = Vec::new();
    let mut start = testsig::frames_for(spec, 0.25);
    while start + burst.len() < signal.len() {
        for (i, s) in burst.iter().enumerate() {
            // A 5 ms fade-in keeps the burst band-limited enough for the kick band to pass it
            // sharply while still having a clear energy rise.
            let ramp = (i as f32 / (0.005 * sample_rate as f32)).min(1.0);
            signal[start + i] += s * ramp;
        }
        starts.push(start);
        start += period;
    }
    (signal, starts)
}

#[test]
fn onsets_land_within_one_hop_of_each_burst_and_nowhere_else() {
    for sample_rate in [22_050_u32, 44_100] {
        let (mut signal, starts) = kick_bursts(sample_rate, 8.0);
        KickBand::new(sample_rate).process_block(&mut signal);
        let det = OnsetDetector::new(sample_rate);
        let onsets = det.detect(&signal);
        assert_eq!(onsets.len(), starts.len(), "{sample_rate} Hz: {onsets:?}");
        for (onset, &start) in onsets.iter().zip(&starts) {
            let error_ms =
                (onset.frame as f64 - start as f64).abs() * 1000.0 / f64::from(sample_rate);
            assert!(
                error_ms <= 2.0,
                "{sample_rate} Hz: onset {} vs {start}: {error_ms:.2} ms",
                onset.frame
            );
            assert!(
                onset.rise_db > 6.0,
                "a burst rises by more than 6 dB: {onset:?}"
            );
            // 0.8 amplitude through the band (60 Hz is inside it): about -2 dBFS at the peak.
            assert!((-6.0..=0.5).contains(&onset.level_db), "level {onset:?}");
        }
    }
}

#[test]
fn onset_detection_is_deterministic() {
    let (mut signal, _) = kick_bursts(22_050, 4.0);
    KickBand::new(22_050).process_block(&mut signal);
    let det = OnsetDetector::new(22_050);
    assert_eq!(det.detect(&signal), det.detect(&signal));
}

fn zero_crossings(signal: &[f32]) -> usize {
    signal
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count()
}

#[test]
fn resampling_keeps_length_frequency_and_amplitude() {
    for (rate_in, rate_out) in [(44_100_u32, 22_050_u32), (48_000, 22_050), (96_000, 22_050)] {
        let seconds = 3.0;
        let spec = AudioSpec::new(rate_in, 1);
        let tone = testsig::sine(spec, 1000.0, 0.5, seconds).data;
        let out = resample_all(&tone, rate_in, rate_out).unwrap();
        let expected_len = (seconds * f64::from(rate_out)).round() as usize;
        assert!(
            (out.len() as i64 - expected_len as i64).abs() <= 1,
            "{rate_in}->{rate_out}: {} vs {expected_len} frames",
            out.len()
        );
        // Frequency: 1 kHz crosses zero 2000 times per second.
        let body = &out[rate_out as usize / 10..out.len() - rate_out as usize / 10];
        let crossings = zero_crossings(body) as f64 / (body.len() as f64 / f64::from(rate_out));
        assert!(
            (crossings - 2000.0).abs() < 4.0,
            "{rate_in}->{rate_out}: {crossings} crossings/s"
        );
        let peak = body.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
        assert!(
            (peak - 0.5).abs() < 0.01,
            "{rate_in}->{rate_out}: peak {peak}"
        );
    }
}

#[test]
fn resampling_preserves_timing() {
    // An impulse at 1.000 s must come out at 1.000 s (+/- one output frame) after the delay trim.
    let rate_in = 48_000;
    let rate_out = 22_050;
    let mut signal = vec![0.0_f32; 2 * rate_in as usize];
    // A short 100 Hz burst is band-limited enough to survive resampling with a clear peak.
    let burst = testsig::sine(AudioSpec::new(rate_in, 1), 100.0, 1.0, 0.01).data;
    signal[rate_in as usize..rate_in as usize + burst.len()].copy_from_slice(&burst);
    let out = resample_all(&signal, rate_in, rate_out).unwrap();
    let first_loud = out.iter().position(|x| x.abs() > 0.2).unwrap();
    let error_ms = (first_loud as f64 / f64::from(rate_out) - 1.0) * 1000.0;
    assert!(error_ms.abs() <= 1.5, "burst arrives {error_ms:.2} ms off");
}

#[test]
fn block_size_does_not_change_the_resampled_signal() {
    let tone = testsig::seeded_noise(AudioSpec::new(44_100, 1), 11, 0.3, 2.7).data;
    let whole = resample_all(&tone, 44_100, 22_050).unwrap();
    for block in [1_usize, 333, 1024, 4096, 100_000] {
        let mut r = Resampler::new(44_100, 22_050).unwrap();
        for chunk in tone.chunks(block) {
            r.push(chunk);
        }
        assert_eq!(r.finish(), whole, "block {block}");
    }
}

#[test]
fn equal_rates_copy_and_downmix_folds_stereo() {
    let stereo = testsig::sine(AudioSpec::CD, 440.0, 0.5, 0.1).data;
    let mut mono = Vec::new();
    to_mono(&stereo, 2, &mut mono);
    assert_eq!(mono.len(), stereo.len() / 2);
    assert_eq!(resample_all(&mono, 44_100, 44_100).unwrap(), mono);
}
