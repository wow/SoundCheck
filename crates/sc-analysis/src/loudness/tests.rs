//! Unit tests of the private parts of `crates/sc-analysis/src/loudness.rs`.
#![allow(clippy::float_cmp)] // exact values are intended in these tests
use super::*;
use approx::assert_abs_diff_eq;
use sc_core::testsig;

#[test]
fn percentile_uses_nearest_rank() {
    assert_eq!(percentile_95(&[]), None);
    assert_eq!(percentile_95(&[-20.0]), Some(-20.0));
    let hundred: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile_95(&hundred), Some(95.0));
    let twenty: Vec<f64> = (1..=20).map(f64::from).collect();
    assert_eq!(percentile_95(&twenty), Some(19.0));
}

#[test]
fn top_power_mean_averages_energies() {
    assert_eq!(top_power_mean(&[-20.0, -30.0], 3), None);
    assert_abs_diff_eq!(
        top_power_mean(&[-20.0, -30.0, -40.0], 2).unwrap(),
        -22.596,
        epsilon = 1e-3
    );
    assert_abs_diff_eq!(
        top_power_mean(&[-23.0; 5], 5).unwrap(),
        -23.0,
        epsilon = 1e-9
    );
}

#[test]
fn momentary_max_is_the_meters_own_reading_after_every_10_ms() {
    // Noise rising to its loudest in the partial last slice, pushed in odd block sizes, must
    // give exactly the maximum a plain meter reads after each 10 ms slice of the same audio,
    // so the ring (whole slices) and the direct read (the partial tail) are both checked.
    for spec in [
        AudioSpec::CD,
        AudioSpec::new(48_000, 1),
        AudioSpec::new(96_000, 2),
    ] {
        let channels = usize::from(spec.channels);
        let mut signal = testsig::seeded_noise(spec, 11, 1.0, 3.456).data;
        let frames = signal.len() / channels;
        let meter = LoudnessMeter::new(spec).unwrap();
        let slice = meter.slice_frames.unwrap();
        assert_ne!(
            frames % meter.hop_frames() % slice,
            0,
            "a partial last slice"
        );
        for (i, sample) in signal.iter_mut().enumerate() {
            // Silence, then a linear rise to full scale at the last frame.
            #[allow(clippy::cast_precision_loss)]
            let ramp = ((i / channels) as f32 / frames as f32 - 0.1).max(0.0);
            *sample *= ramp;
        }

        let mut reference = EbuR128::new(
            u32::from(spec.channels),
            spec.sample_rate,
            Mode::I | Mode::LRA,
        )
        .unwrap();
        if channels == 1 {
            reference.set_channel(0, Channel::DualMono).unwrap();
        }
        let mut expected = f64::NEG_INFINITY;
        let mut last = f64::NEG_INFINITY;
        for part in signal.chunks(slice * channels) {
            reference.add_frames_f32(part).unwrap();
            last = loudness_or_silence(reference.loudness_momentary());
            expected = expected.max(last);
        }
        assert_eq!(expected, last, "the loudest window ends the stream");

        let mut meter = meter;
        // 1234 samples: whole frames for mono and stereo, never a whole hop.
        for block in signal.chunks(1234) {
            meter.push(block);
        }
        let max = meter.finish().momentary_max.unwrap().0;
        assert_abs_diff_eq!(max, expected, epsilon = 1e-9);
    }
}

#[test]
fn rates_without_whole_10_ms_slices_read_the_meter_directly() {
    assert_eq!(
        LoudnessMeter::new(AudioSpec::new(22_050, 2))
            .unwrap()
            .slice_frames,
        None
    );
    assert_eq!(
        LoudnessMeter::new(AudioSpec::CD).unwrap().slice_frames,
        Some(441)
    );
}

#[test]
fn rejects_more_than_two_channels() {
    let err = LoudnessMeter::new(AudioSpec::new(44_100, 3)).unwrap_err();
    assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
    assert_eq!(
        LoudnessMeter::new(AudioSpec::CD).unwrap().hop_frames(),
        4_410
    );
}
