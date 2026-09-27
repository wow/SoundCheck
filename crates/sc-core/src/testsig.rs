//! Deterministic synthetic signals for tests and benches (feature `testsig`).
//!
//! Every generator is pure: the same arguments always produce the same samples, so goldens and
//! tolerance tests are reproducible on every platform.

use std::f64::consts::TAU;

use crate::audio::{AudioBuffer, AudioSpec};
use crate::units::{Bpm, SampleIndex};

/// Number of frames for `seconds` at the spec's rate, rounded to nearest.
#[must_use]
// Rounded and non-negative before the cast.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn frames_for(spec: AudioSpec, seconds: f64) -> usize {
    (seconds * f64::from(spec.sample_rate)).round().max(0.0) as usize
}

/// A full-scale-relative sine on every channel.
#[must_use]
// Frame indices are far below 2^53.
#[allow(clippy::cast_precision_loss)]
pub fn sine(spec: AudioSpec, freq_hz: f64, amplitude: f32, seconds: f64) -> AudioBuffer {
    let frames = frames_for(spec, seconds);
    let channels = usize::from(spec.channels);
    let step = TAU * freq_hz / f64::from(spec.sample_rate);
    let mut data = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        // Computed in f64, narrowed once per sample: the rounding is the expected f32 quantisation.
        #[allow(clippy::cast_possible_truncation)]
        let s = ((n as f64 * step).sin() * f64::from(amplitude)) as f32;
        data.extend(std::iter::repeat_n(s, channels));
    }
    AudioBuffer::new(spec, data)
}

/// Silence with a single unit impulse at `at` on every channel.
#[must_use]
// A test signal position never exceeds usize.
#[allow(clippy::cast_possible_truncation)]
pub fn impulse(spec: AudioSpec, at: SampleIndex, seconds: f64) -> AudioBuffer {
    let mut buf = AudioBuffer::silence(spec, frames_for(spec, seconds));
    let channels = usize::from(spec.channels);
    let frame = at.0 as usize;
    if frame < buf.frames() {
        for ch in 0..channels {
            buf.data[frame * channels + ch] = 1.0;
        }
    }
    buf
}

/// A metronome: a 1 kHz burst of `click_ms` at every beat, the first beat of each 4/4 bar louder.
///
/// Beat `i` starts exactly at frame `round(i * 60 / bpm * sample_rate)`, which is what grid tests
/// compare against.
#[must_use]
pub fn click_track(spec: AudioSpec, bpm: Bpm, beats: u32, click_ms: f64) -> AudioBuffer {
    let spb = 60.0 / bpm.0;
    let total_seconds = spb * f64::from(beats) + click_ms / 1000.0;
    let mut buf = AudioBuffer::silence(spec, frames_for(spec, total_seconds));
    let channels = usize::from(spec.channels);
    let click_frames = frames_for(spec, click_ms / 1000.0);
    let burst = sine(spec, 1000.0, 1.0, click_ms / 1000.0);
    for beat in 0..beats {
        let start = frames_for(spec, spb * f64::from(beat));
        let amp = if beat % 4 == 0 { 1.0 } else { 0.7 };
        for n in 0..click_frames {
            for ch in 0..channels {
                let idx = (start + n) * channels + ch;
                if idx < buf.data.len() {
                    buf.data[idx] = burst.data[n * channels + ch] * amp;
                }
            }
        }
    }
    buf
}

/// Uniform white noise in `-amplitude..amplitude` from a seeded generator.
#[must_use]
pub fn seeded_noise(spec: AudioSpec, seed: u64, amplitude: f32, seconds: f64) -> AudioBuffer {
    let mut rng = Rng::new(seed);
    let len = frames_for(spec, seconds) * usize::from(spec.channels);
    let data = (0..len).map(|_| rng.next_unit() * amplitude).collect();
    AudioBuffer::new(spec, data)
}

/// A tiny xorshift64* generator; deterministic across platforms, not for anything but tests.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator seeded with `seed` (zero is remapped to a fixed non-zero state).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Next value uniformly in `-1.0..1.0`.
    // Only the top 24 bits are used, which f32 represents exactly.
    #[allow(clippy::cast_precision_loss)]
    pub fn next_unit(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as f32; // 0 ..= 2^24 - 1
        bits / 8_388_608.0 - 1.0
    }
}

#[cfg(test)]
mod tests;
