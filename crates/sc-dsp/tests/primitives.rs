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

/// `seconds` of mono at `sample_rate`: an optional sustained 50 Hz bass (`bass` amplitude) and a
/// 60 Hz kick of 40 ms at 0.8 starting `kick_ms` in, both raised from silence by a 2 ms
/// raised-cosine fade at the start of the file, as a Prepare cut leaves it, in a quiet noise floor.
fn cut_file(sample_rate: u32, kick_ms: f64, bass: f32, seconds: f64) -> (Vec<f32>, usize) {
    let spec = AudioSpec::new(sample_rate, 1);
    let mut signal = testsig::seeded_noise(spec, 5, 0.002, seconds).data;
    if bass > 0.0 {
        let tone = testsig::sine(spec, 50.0, bass, seconds).data;
        for (s, t) in signal.iter_mut().zip(&tone) {
            *s += t;
        }
    }
    let start = testsig::frames_for(spec, kick_ms / 1000.0);
    let burst = testsig::sine(spec, 60.0, 0.8, 0.04).data;
    for (i, s) in burst.iter().enumerate() {
        let ramp = (i as f32 / (0.005 * sample_rate as f32)).min(1.0);
        signal[start + i] += s * ramp;
    }
    // A later kick, so the file has more than one attack, as music does.
    let later = testsig::frames_for(spec, 0.5);
    for (i, s) in burst.iter().enumerate() {
        let ramp = (i as f32 / (0.005 * sample_rate as f32)).min(1.0);
        signal[later + i] += s * ramp;
    }
    let fade = testsig::frames_for(spec, 0.002);
    for (i, s) in signal.iter_mut().take(fade).enumerate() {
        let x = std::f32::consts::PI * i as f32 / fade as f32;
        *s *= 0.5 * (1.0 - x.cos());
    }
    (signal, start)
}

/// The first onset of `signal` (kick band applied) relative to `start`, in ms.
fn first_onset_error_ms(mut signal: Vec<f32>, sample_rate: u32, start: usize) -> (f64, usize) {
    KickBand::new(sample_rate).process_block(&mut signal);
    let onsets = OnsetDetector::new(sample_rate).detect(&signal);
    let first = onsets.first().expect("an onset");
    let error_ms = (first.frame as f64 - start as f64) * 1000.0 / f64::from(sample_rate);
    (error_ms, onsets.len())
}

#[test]
fn onsets_in_the_first_45_ms_are_found() {
    // A Prepare cut leaves bar 1 a lead of 5 to 15 ms after the start of the file; its kick must
    // be found there, and the fade-in is no attack. Over silence: within the 2 ms the burst
    // tests allow. Over a bass that was sounding when the file was cut: within one hop of where
    // the same kick is placed in the middle of the file over the bass at the same phase (a bass
    // at -18 dB under the kick delays the start the detector reads by up to 3.5 ms anywhere).
    for sample_rate in [22_050_u32, 44_100] {
        for bass in [0.0_f32, 0.1] {
            for kick_ms in [5.0, 6.0, 10.0, 15.0, 29.0, 40.0, 44.0] {
                let what = format!("{sample_rate} Hz, bass {bass}, kick at {kick_ms} ms");
                let (signal, start) = cut_file(sample_rate, kick_ms, bass, 1.0);
                let (error_ms, count) = first_onset_error_ms(signal, sample_rate, start);
                assert_eq!(count, 2, "{what}: one onset per kick");
                if bass == 0.0 {
                    assert!(error_ms.abs() <= 2.0, "{what}: {error_ms:+.2} ms");
                } else {
                    // 200 ms later the 50 Hz bass is at the same phase.
                    let (signal, start) = cut_file(sample_rate, kick_ms + 200.0, bass, 1.0);
                    let (inside_ms, _) = first_onset_error_ms(signal, sample_rate, start);
                    assert!(
                        (error_ms - inside_ms).abs() <= 1.0 && error_ms.abs() <= 3.5,
                        "{what}: {error_ms:+.2} ms, {inside_ms:+.2} ms in the middle"
                    );
                }
            }
        }
    }
}

#[test]
fn a_file_starting_on_a_sustained_tone_has_no_onset_at_its_start() {
    // A cut in the middle of a held bass, at a zero crossing: the fade-in and the first quarter
    // cycle raise it, but nothing attacks.
    for sample_rate in [22_050_u32, 44_100] {
        for freq_hz in [30.0, 40.0, 50.0, 60.0, 80.0, 100.0, 140.0] {
            let spec = AudioSpec::new(sample_rate, 1);
            let mut signal = testsig::sine(spec, freq_hz, 0.5, 1.0).data;
            let fade = testsig::frames_for(spec, 0.002);
            for (i, s) in signal.iter_mut().take(fade).enumerate() {
                let x = std::f32::consts::PI * i as f32 / fade as f32;
                *s *= 0.5 * (1.0 - x.cos());
            }
            KickBand::new(sample_rate).process_block(&mut signal);
            assert_eq!(
                OnsetDetector::new(sample_rate).detect(&signal),
                Vec::new(),
                "{sample_rate} Hz, {freq_hz} Hz tone"
            );
        }
    }
}

/// Fade-in shapes, as gain at `x` (0 to 1 of the fade).
#[derive(Debug, Clone, Copy)]
enum Fade {
    Linear,
    Cosine,
    /// Linear in dB, from -60 dB.
    Exponential,
}

impl Fade {
    fn gain(self, x: f32) -> f32 {
        match self {
            Self::Linear => x,
            Self::Cosine => 0.5 * (1.0 - (std::f32::consts::PI * x).cos()),
            Self::Exponential => 10_f32.powf(-3.0 * (1.0 - x)),
        }
    }
}

#[test]
fn a_fade_in_over_a_held_bass_is_no_attack() {
    // A track that fades in over a held bass, with kicks from 300 ms on: the fade is not an
    // attack at the start, whatever its length and shape, and every kick is still found.
    let mut failures = Vec::new();
    for sample_rate in [22_050_u32, 44_100] {
        let spec = AudioSpec::new(sample_rate, 1);
        for freq_hz in [40.0, 60.0, 80.0] {
            for fade_ms in [5.0, 10.0, 20.0, 50.0, 100.0] {
                for shape in [Fade::Linear, Fade::Cosine, Fade::Exponential] {
                    let what =
                        format!("{sample_rate} Hz, {freq_hz} Hz bass, {fade_ms} ms {shape:?}");
                    let mut signal = testsig::seeded_noise(spec, 9, 0.002, 2.0).data;
                    let bass = testsig::sine(spec, freq_hz, 0.3, 2.0).data;
                    let fade = testsig::frames_for(spec, fade_ms / 1000.0);
                    for (i, (s, b)) in signal.iter_mut().zip(&bass).enumerate() {
                        let x = (i as f32 / fade as f32).min(1.0);
                        *s = (*s + b) * shape.gain(x);
                    }
                    let burst = testsig::sine(spec, 60.0, 0.8, 0.04).data;
                    let mut kicks = Vec::new();
                    for k in 0..3 {
                        let start = testsig::frames_for(spec, 0.3 + 0.5 * f64::from(k));
                        for (i, b) in burst.iter().enumerate() {
                            let ramp = (i as f32 / (0.005 * sample_rate as f32)).min(1.0);
                            signal[start + i] += b * ramp;
                        }
                        kicks.push(start);
                    }
                    KickBand::new(sample_rate).process_block(&mut signal);
                    let onsets = OnsetDetector::new(sample_rate).detect(&signal);
                    // Nothing in the start window, where the start rules apply (a 60 dB
                    // exponential fade over 100 ms still rises 10 dB within 10 ms after it,
                    // which is an attack anywhere in a file).
                    let window = sc_dsp::onset::START_WINDOW_MS * testsig::frames_for(spec, 0.001);
                    let early = onsets.iter().any(|o| o.frame < window);
                    let missed = kicks.iter().any(|&k| {
                        !onsets
                            .iter()
                            .any(|o| o.frame.abs_diff(k) <= testsig::frames_for(spec, 0.003))
                    });
                    if early || missed {
                        failures.push(format!("{what}: {onsets:?}"));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
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
