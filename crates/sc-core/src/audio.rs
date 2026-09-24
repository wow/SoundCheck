//! Audio buffers: interleaved `f32` PCM with an explicit [`AudioSpec`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::units::Seconds;

/// Sample rate and channel count of a buffer or stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AudioSpec {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
}

impl AudioSpec {
    /// 44.1 kHz stereo.
    pub const CD: Self = Self {
        sample_rate: 44_100,
        channels: 2,
    };

    /// A new spec.
    #[must_use]
    pub const fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }
}

/// Interleaved `f32` PCM. Invariant: `data.len()` is a multiple of `spec.channels`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    /// Rate and channel layout of `data`.
    pub spec: AudioSpec,
    /// Interleaved samples, nominally in `-1.0..=1.0`.
    pub data: Vec<f32>,
}

impl AudioBuffer {
    /// Wraps interleaved samples.
    ///
    /// # Panics
    /// If `spec.channels` is zero or `data.len()` is not a multiple of the channel count; both are
    /// programming errors, not user-file errors.
    #[must_use]
    pub fn new(spec: AudioSpec, data: Vec<f32>) -> Self {
        assert!(
            spec.channels > 0,
            "an AudioBuffer needs at least one channel"
        );
        assert!(
            data.len().is_multiple_of(usize::from(spec.channels)),
            "interleaved data length must be a multiple of the channel count"
        );
        Self { spec, data }
    }

    /// A buffer of `frames` frames of digital silence.
    #[must_use]
    pub fn silence(spec: AudioSpec, frames: usize) -> Self {
        Self::new(spec, vec![0.0; frames * usize::from(spec.channels)])
    }

    /// Number of frames (samples per channel).
    #[must_use]
    pub fn frames(&self) -> usize {
        self.data.len() / usize::from(self.spec.channels)
    }

    /// Duration derived from the frame count.
    #[must_use]
    // A frame count never exceeds 2^53 in practice.
    #[allow(clippy::cast_precision_loss)]
    pub fn duration(&self) -> Seconds {
        Seconds(self.frames() as f64 / f64::from(self.spec.sample_rate))
    }

    /// Iterates over one channel (0-based).
    ///
    /// # Panics
    /// If `channel` is out of range.
    pub fn channel(&self, channel: usize) -> impl Iterator<Item = f32> + '_ {
        let channels = usize::from(self.spec.channels);
        assert!(
            channel < channels,
            "channel {channel} out of range for {channels} channels"
        );
        self.data.iter().skip(channel).step_by(channels).copied()
    }

    /// Largest absolute sample value in the buffer (0.0 for an empty buffer).
    #[must_use]
    pub fn peak_abs(&self) -> f32 {
        self.data.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact values are intended in these tests
    use super::*;

    #[test]
    fn frames_duration_and_channels() {
        let buf = AudioBuffer::new(AudioSpec::new(4, 2), vec![0.1, -0.5, 0.2, 0.3]);
        assert_eq!(buf.frames(), 2);
        assert_eq!(buf.duration(), Seconds(0.5));
        assert_eq!(buf.channel(1).collect::<Vec<_>>(), vec![-0.5, 0.3]);
        assert_eq!(buf.peak_abs(), 0.5);
    }

    #[test]
    fn silence_has_the_requested_length() {
        let buf = AudioBuffer::silence(AudioSpec::CD, 10);
        assert_eq!(buf.frames(), 10);
        assert_eq!(buf.data.len(), 20);
        assert_eq!(buf.peak_abs(), 0.0);
    }

    #[test]
    #[should_panic(expected = "multiple of the channel count")]
    fn rejects_ragged_interleaving() {
        let _ = AudioBuffer::new(AudioSpec::CD, vec![0.0; 3]);
    }
}
