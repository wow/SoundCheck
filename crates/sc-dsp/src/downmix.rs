//! Mono fold-down of interleaved audio: stereo becomes `(L + R) / 2` (-6 dB, so a correlated
//! signal keeps its level and full-scale input cannot clip), mono is copied.

/// Appends the mono fold-down of an interleaved block to `out`.
///
/// # Panics
/// When `channels` is 0 or the block is not whole frames.
pub fn to_mono(interleaved: &[f32], channels: u16, out: &mut Vec<f32>) {
    let channels = usize::from(channels);
    assert!(channels > 0, "at least one channel");
    assert_eq!(interleaved.len() % channels, 0, "whole frames");
    match channels {
        1 => out.extend_from_slice(interleaved),
        2 => out.extend(
            interleaved
                .as_chunks::<2>()
                .0
                .iter()
                .map(|frame| f32::midpoint(frame[0], frame[1])),
        ),
        n => {
            // Multichannel is refused upstream; folding evenly keeps the function total.
            // Channel counts are tiny, so the cast is exact.
            #[allow(clippy::cast_precision_loss)]
            let scale = 1.0 / n as f32;
            out.extend(
                interleaved
                    .chunks_exact(n)
                    .map(|frame| frame.iter().sum::<f32>() * scale),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_averages_and_mono_copies() {
        let mut out = Vec::new();
        to_mono(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.0]);
        to_mono(&[0.25, -0.25], 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.0, 0.25, -0.25]);
    }
}
