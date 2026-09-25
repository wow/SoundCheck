//! Loudness per ITU-R BS.1770-5 and EBU R 128 over a stream of interleaved `f32`.
//!
//! Gated integrated loudness, the 400 ms momentary and 3 s short-term windows, loudness range
//! (EBU Tech 3342) and the 4x-oversampled true peak (BS.1770-5 Annex 2) come from the `ebur128`
//! crate, a port of libebur128 that passes the EBU Tech 3341/3342 test set. On top of it this
//! module keeps what the standards do not define: the short-term timeline at a 100 ms hop,
//! S-P95 (the DJ alignment statistic), S-top30 and the peak-to-loudness ratio.
//!
//! Mono is measured as dual mono (+3.01 LU against one channel), as Tech 3341 prescribes.
//!
//! Determinism: audio reaches the meter in exact 100 ms chunks whatever block sizes the caller
//! pushes, so two runs and any chunking yield identical numbers.

use ebur128::{Channel, EbuR128, Mode};
use sc_core::analysis::{LoudnessReport, TIMELINE_HOP_MS, Timeline};
use sc_core::{AudioBuffer, AudioSpec, DbFs, DbTp, Error, Lu, Lufs, Result};

/// Absolute gate of BS.1770-5: windows at or below it are silence for every statistic.
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;

/// Number of 100 ms hops in the 30 s that S-top30 averages.
const TOP30_HOPS: usize = 300;

/// A streaming loudness meter: [`push`](Self::push) blocks, then [`finish`](Self::finish).
pub struct LoudnessMeter {
    meter: EbuR128,
    spec: AudioSpec,
    /// Interleaved samples per 100 ms hop.
    hop_samples: usize,
    pending: Vec<f32>,
    short_term: Vec<Option<f32>>,
    momentary_max: f64,
    short_term_max: f64,
}

impl std::fmt::Debug for LoudnessMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoudnessMeter")
            .field("spec", &self.spec)
            .field("hops", &self.short_term.len())
            .finish_non_exhaustive()
    }
}

impl LoudnessMeter {
    /// A meter for `spec`.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for zero or more than two channels, or a sample rate the
    /// meter cannot run at.
    pub fn new(spec: AudioSpec) -> Result<Self> {
        if !(1..=2).contains(&spec.channels) {
            return Err(Error::InvalidArgument(format!(
                "{} channels; the loudness meter takes mono or stereo",
                spec.channels
            )));
        }
        let mode = Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::SAMPLE_PEAK;
        let mut meter =
            EbuR128::new(u32::from(spec.channels), spec.sample_rate, mode).map_err(|e| {
                Error::InvalidArgument(format!("loudness meter at {} Hz: {e}", spec.sample_rate))
            })?;
        if spec.channels == 1 {
            meter
                .set_channel(0, Channel::DualMono)
                .map_err(|e| Error::InvalidArgument(format!("dual mono: {e}")))?;
        }
        let hop_frames = usize::try_from(spec.sample_rate / 10)
            .map_err(|_| Error::InvalidArgument("sample rate does not fit usize".into()))?;
        let hop_samples = hop_frames * usize::from(spec.channels);
        Ok(Self {
            meter,
            spec,
            hop_samples,
            pending: Vec::with_capacity(hop_samples),
            short_term: Vec::new(),
            momentary_max: f64::NEG_INFINITY,
            short_term_max: f64::NEG_INFINITY,
        })
    }

    /// The spec this meter was built for.
    #[must_use]
    pub fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Frames per timeline hop (100 ms).
    #[must_use]
    pub fn hop_frames(&self) -> usize {
        self.hop_samples / usize::from(self.spec.channels)
    }

    /// Feeds interleaved samples; any block size, whole frames.
    pub fn push(&mut self, block: &[f32]) {
        debug_assert_eq!(
            block.len() % usize::from(self.spec.channels),
            0,
            "whole frames"
        );
        let mut rest = block;
        while !rest.is_empty() {
            let need = self.hop_samples - self.pending.len();
            let take = need.min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == self.hop_samples {
                let chunk = std::mem::take(&mut self.pending);
                self.feed_hop(&chunk);
                self.pending = chunk;
                self.pending.clear();
            }
        }
    }

    fn feed_hop(&mut self, chunk: &[f32]) {
        self.meter
            .add_frames_f32(chunk)
            .expect("a hop is whole frames of the configured channel count");
        let momentary = loudness_or_silence(self.meter.loudness_momentary());
        let short_term = loudness_or_silence(self.meter.loudness_shortterm());
        self.momentary_max = self.momentary_max.max(momentary);
        self.short_term_max = self.short_term_max.max(short_term);
        // Timeline values are display precision; f32 is plenty.
        #[allow(clippy::cast_possible_truncation)]
        let gated = (short_term > ABSOLUTE_GATE_LUFS).then_some(short_term as f32);
        self.short_term.push(gated);
    }

    /// Closes the stream and computes the report.
    #[must_use]
    pub fn finish(mut self) -> LoudnessReport {
        self.feed_tail();
        let integrated = finite_lufs(loudness_or_silence(self.meter.loudness_global()));
        let gated: Vec<f64> = self
            .short_term
            .iter()
            .flatten()
            .map(|&s| f64::from(s))
            .collect();
        let lra = (gated.len() >= 2).then(|| Lu(self.meter.loudness_range().unwrap_or(0.0)));
        let short_term_p95 = percentile_95(&gated).map(Lufs);
        let short_term_top30 = top_power_mean(&gated, TOP30_HOPS).map(Lufs);
        let channels = u32::from(self.spec.channels);
        let sample_peak_linear = (0..channels)
            .map(|ch| self.meter.sample_peak(ch).unwrap_or(0.0))
            .fold(0.0_f64, f64::max);
        let true_peak_linear = (0..channels)
            .map(|ch| self.meter.true_peak(ch).unwrap_or(0.0))
            .fold(sample_peak_linear, f64::max);
        let sample_peak = DbFs::from_linear(sample_peak_linear);
        let true_peak = DbTp(DbFs::from_linear(true_peak_linear).0);
        let plr = integrated.map(|i| Lu(true_peak.0 - i.0));
        LoudnessReport {
            integrated,
            momentary_max: finite_lufs(self.momentary_max),
            short_term_max: finite_lufs(self.short_term_max),
            short_term_p95,
            short_term_top30,
            lra,
            true_peak,
            sample_peak,
            plr,
            dual_mono: self.spec.channels == 1,
            timeline: Timeline {
                hop_ms: TIMELINE_HOP_MS,
                short_term: self.short_term,
            },
        }
    }

    /// The trailing partial hop still counts for the peaks and completes the meter's own gating
    /// blocks; it adds no timeline entry.
    fn feed_tail(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let tail = std::mem::take(&mut self.pending);
        self.meter
            .add_frames_f32(&tail)
            .expect("the tail is whole frames of the configured channel count");
    }
}

/// Measures a whole buffer; a convenience over [`LoudnessMeter`].
///
/// # Errors
/// As for [`LoudnessMeter::new`].
pub fn measure(buf: &AudioBuffer) -> Result<LoudnessReport> {
    let mut meter = LoudnessMeter::new(buf.spec)?;
    meter.push(&buf.data);
    Ok(meter.finish())
}

/// The meter's readings; an error (impossible once the mode is set) reads as silence.
fn loudness_or_silence(reading: std::result::Result<f64, ebur128::Error>) -> f64 {
    reading.unwrap_or(f64::NEG_INFINITY)
}

fn finite_lufs(value: f64) -> Option<Lufs> {
    value.is_finite().then_some(Lufs(value))
}

/// 95th percentile by the nearest-rank method (the value at rank `ceil(0.95 n)`).
fn percentile_95(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (95 * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1])
}

/// Power mean (in LUFS) of the `count` largest values; `None` when there are fewer than `count`.
fn top_power_mean(values: &[f64], count: usize) -> Option<f64> {
    if values.len() < count || count == 0 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let energy: f64 = sorted[..count]
        .iter()
        .map(|lufs| 10.0_f64.powf(lufs / 10.0))
        .sum();
    // `count` is a small window count, exactly representable.
    #[allow(clippy::cast_precision_loss)]
    let mean = energy / count as f64;
    Some(10.0 * mean.log10())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact values are intended in these tests
    use super::*;
    use approx::assert_abs_diff_eq;

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
    fn rejects_more_than_two_channels() {
        let err = LoudnessMeter::new(AudioSpec::new(44_100, 3)).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
        assert_eq!(
            LoudnessMeter::new(AudioSpec::CD).unwrap().hop_frames(),
            4_410
        );
    }
}
