//! The streaming resampler for playback: the same output as the whole-clip converter per channel,
//! whatever the block sizes, and a reset that starts over as a fresh converter would.

use sc_dsp::StreamResampler;
use sc_dsp::resample::resample_all;

/// A stereo test signal: a 440 Hz sine on the left, 1 kHz on the right.
fn stereo(frames: usize, rate: f64) -> Vec<f32> {
    (0..frames)
        .flat_map(|n| {
            #[allow(clippy::cast_precision_loss)]
            let t = n as f64 / rate;
            #[allow(clippy::cast_possible_truncation)]
            [
                (0.5 * (std::f64::consts::TAU * 440.0 * t).sin()) as f32,
                (0.3 * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32,
            ]
        })
        .collect()
}

fn stream(input: &[f32], blocks: &[usize], rate_in: u32, rate_out: u32) -> Vec<f32> {
    let mut r = StreamResampler::new(rate_in, rate_out, 2).unwrap();
    let mut out = Vec::new();
    let mut rest = input;
    for &b in blocks.iter().cycle() {
        if rest.is_empty() {
            break;
        }
        let take = (2 * b).min(rest.len());
        r.process(&rest[..take], &mut out);
        rest = &rest[take..];
    }
    out
}

fn channel(interleaved: &[f32], c: usize) -> Vec<f32> {
    interleaved.iter().skip(c).step_by(2).copied().collect()
}

#[test]
fn streaming_matches_the_whole_clip_converter_per_channel() {
    let input = stereo(44_100, 44_100.0);
    let out = stream(&input, &[1024], 44_100, 48_000);
    assert!(out.len() / 2 > 44_000, "{} frames ready", out.len() / 2);
    for c in 0..2 {
        let whole = resample_all(&channel(&input, c), 44_100, 48_000).unwrap();
        let streamed = channel(&out, c);
        let worst = streamed
            .iter()
            .zip(&whole)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 1e-5, "channel {c}: {worst}");
    }
}

#[test]
fn block_sizes_do_not_change_the_output() {
    let input = stereo(20_000, 44_100.0);
    let a = stream(&input, &[1024], 44_100, 48_000);
    let b = stream(&input, &[1, 777, 4096, 13], 44_100, 48_000);
    assert_eq!(a, b);
}

#[test]
fn equal_rates_copy_and_a_reset_starts_over() {
    let input = stereo(5000, 48_000.0);
    assert_eq!(stream(&input, &[300], 48_000, 48_000), input);

    let mut r = StreamResampler::new(44_100, 48_000, 2).unwrap();
    let mut before = Vec::new();
    r.process(&stereo(3000, 44_100.0), &mut before);
    r.reset();
    let fresh_input = stereo(10_000, 44_100.0);
    let mut after = Vec::new();
    r.process(&fresh_input, &mut after);
    assert_eq!(after, stream(&fresh_input, &[1024], 44_100, 48_000));
    assert!(StreamResampler::new(0, 48_000, 2).is_err());
    assert!(StreamResampler::new(44_100, 48_000, 0).is_err());
}

#[test]
fn a_flush_lets_the_last_frames_out() {
    let input = stereo(3000, 44_100.0);
    let mut r = StreamResampler::new(44_100, 48_000, 2).unwrap();
    let mut out = Vec::new();
    r.process(&input, &mut out);
    let before = out.len() / 2;
    r.flush(&mut out);
    let whole = resample_all(&channel(&input, 0), 44_100, 48_000).unwrap();
    assert!(before < whole.len(), "{before} frames before the flush");
    assert!(
        out.len() / 2 >= whole.len(),
        "{} frames after",
        out.len() / 2
    );
    let left = channel(&out, 0);
    let worst = left
        .iter()
        .zip(&whole)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(worst < 1e-5, "{worst}");
}
