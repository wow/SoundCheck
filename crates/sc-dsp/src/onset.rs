//! Energy-rise onset detection at 1 ms resolution on a mono signal (normally the kick band).
//!
//! The rectified signal goes through an envelope follower with instant attack and a
//! [`RELEASE_MS`] release, so the envelope keeps the attack's timing to the sample while a
//! sustained low tone, whose 1 ms energy would otherwise swing by tens of dB every half-cycle,
//! stays flat. The envelope is sampled per 1 ms hop and converted to dB; the rise of each hop over
//! the lowest level of the preceding [`RISE_WINDOW_MS`] is peak-picked: a hop is an attack when
//! its rise is the largest within +/-[`MIN_SPACING_MS`], exceeds `median + k * MAD` of all rises
//! (a robust threshold that ignores the small fluctuations of a sustained band) and exceeds
//! [`MIN_RISE_DB`], at a level within [`LEVEL_GATE_DB`] of the envelope's 95th percentile. The
//! onset is placed at the first hop of that rise whose single step reaches half of its steepest
//! step, the start of the attack. The method
//! is the classic energy-flux onset function (Bello et al., "A tutorial on onset detection in
//! music signals", IEEE TSAP 2005, section III-A) restricted to one band.

/// Window after an onset over which its level is read.
pub const LEVEL_WINDOW_MS: usize = 20;

/// Span over which an attack's rise is measured.
pub const RISE_WINDOW_MS: usize = 10;

/// An attack must reach within this many dB of the envelope's 95th percentile; quieter rises are
/// the noise floor breathing.
pub const LEVEL_GATE_DB: f64 = 40.0;

/// Onsets closer than this are merged into the stronger one.
pub const MIN_SPACING_MS: usize = 30;

/// Multiplier of the median absolute deviation above the median rise.
const MAD_FACTOR: f64 = 3.0;

/// Smallest rise that counts as an attack, in dB per hop; a kick rises by tens of dB within a
/// millisecond, while a sustained band fluctuates by a few dB.
pub const MIN_RISE_DB: f64 = 6.0;

/// Release time of the envelope follower; longer than a 30 Hz half-cycle (16.7 ms).
pub const RELEASE_MS: f64 = 40.0;

/// Floor added before the logarithm, well below any audible level (about -140 dB).
const AMPLITUDE_FLOOR: f64 = 1e-7;

/// An energy rise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Onset {
    /// First frame of the hop in which the rise happens, at the input sample rate.
    pub frame: usize,
    /// The rise in dB from the previous hop; larger is a sharper attack.
    pub rise_db: f32,
    /// The envelope's peak over the [`LEVEL_WINDOW_MS`] after the rise, in dBFS; how loud the
    /// attack is (the accent).
    pub level_db: f32,
}

/// A detector for one sample rate.
#[derive(Debug, Clone)]
pub struct OnsetDetector {
    hop_frames: usize,
    release: f64,
}

impl OnsetDetector {
    /// A detector with 1 ms hops (rounded to whole frames) at `sample_rate`.
    ///
    /// # Panics
    /// When `sample_rate` is below 1 kHz.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let hop = (f64::from(sample_rate) / 1000.0).round();
        assert!(hop >= 1.0, "sample rate {sample_rate} Hz is below 1 kHz");
        // `hop` is a small positive integer-valued float.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let hop_frames = hop as usize;
        let release = (-1000.0 / (RELEASE_MS * f64::from(sample_rate))).exp();
        Self {
            hop_frames,
            release,
        }
    }

    /// Frames per hop.
    #[must_use]
    pub fn hop_frames(&self) -> usize {
        self.hop_frames
    }

    /// The envelope in dB, one value per whole hop: the follower's peak within the hop.
    #[must_use]
    pub fn envelope_db(&self, mono: &[f32]) -> Vec<f64> {
        // Start the follower at the first hop's level: a file that opens on a floor of noise
        // must not read as an attack at frame 0 (an onset inside the very first hop is missed).
        let mut env = mono
            .iter()
            .take(self.hop_frames)
            .fold(0.0_f64, |m, &x| m.max(f64::from(x).abs()));
        mono.chunks_exact(self.hop_frames)
            .map(|hop| {
                let mut peak = 0.0_f64;
                for &x in hop {
                    let x = f64::from(x).abs();
                    env = if x > env { x } else { env * self.release };
                    peak = peak.max(env);
                }
                20.0 * (peak + AMPLITUDE_FLOOR).log10()
            })
            .collect()
    }

    /// Onsets in `mono`, in ascending frame order. The first [`MIN_SPACING_MS`] are warm-up and
    /// never contain one.
    #[must_use]
    pub fn detect(&self, mono: &[f32]) -> Vec<Onset> {
        let envelope = self.envelope_db(mono);
        let n = envelope.len();
        if n < RISE_WINDOW_MS + 2 {
            return Vec::new();
        }
        // Rise over the preceding window: the level now above the lowest level before it. A
        // band-limited attack spreads over several milliseconds; measured per hop it would
        // stay below any threshold in a dense mix.
        let rises: Vec<f64> = (0..n)
            .map(|k| {
                let lo = k.saturating_sub(RISE_WINDOW_MS);
                let floor = envelope[lo..k]
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min);
                if floor.is_finite() {
                    (envelope[k] - floor).max(0.0)
                } else {
                    0.0
                }
            })
            .collect();
        let threshold = robust_threshold(&rises).max(MIN_RISE_DB);
        let gate = {
            let mut sorted = envelope.clone();
            sorted.sort_by(f64::total_cmp);
            sorted[(95 * (n - 1)) / 100] - LEVEL_GATE_DB
        };
        let spacing = MIN_SPACING_MS.max(1);
        let mut onsets = Vec::new();
        for (k, &rise) in rises.iter().enumerate() {
            if k < spacing || rise <= threshold || envelope[k] < gate {
                continue;
            }
            let lo = k - spacing;
            let hi = (k + spacing + 1).min(n);
            let is_local_max = rises[lo..hi].iter().enumerate().all(|(j, &other)| {
                if lo + j < k {
                    other < rise
                } else {
                    other <= rise
                }
            });
            if !is_local_max {
                continue;
            }
            // The attack starts at the first hop of the rise whose step reaches half of the
            // rise's steepest step.
            let first = k.saturating_sub(RISE_WINDOW_MS - 1).max(1);
            let steps: Vec<f64> = (first..=k).map(|j| envelope[j] - envelope[j - 1]).collect();
            let steepest = steps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let start = first + steps.iter().position(|&d| d >= 0.5 * steepest).unwrap_or(0);
            let end = (start + LEVEL_WINDOW_MS).min(n);
            let level = envelope[start..end]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            // Rises and levels are dB values of modest size; f32 keeps every meaningful digit.
            #[allow(clippy::cast_possible_truncation)]
            let (rise_db, level_db) = (rise as f32, level as f32);
            onsets.push(Onset {
                frame: start * self.hop_frames,
                rise_db,
                level_db,
            });
        }
        onsets
    }
}

/// `median + MAD_FACTOR * MAD` of the positive rises; zero when nothing rises.
fn robust_threshold(rises: &[f64]) -> f64 {
    let positive: Vec<f64> = rises.iter().copied().filter(|&r| r > 0.0).collect();
    if positive.is_empty() {
        return 0.0;
    }
    let center = median(&positive);
    let deviations: Vec<f64> = positive.iter().map(|r| (r - center).abs()).collect();
    let mad = median(&deviations);
    center + MAD_FACTOR * mad
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        f64::midpoint(sorted[n / 2 - 1], sorted[n / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_and_even_lengths() {
        assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-12);
        assert!((median(&[4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < 1e-12);
    }

    #[test]
    fn silence_has_no_onsets() {
        let det = OnsetDetector::new(22_050);
        assert_eq!(det.hop_frames(), 22);
        assert!(det.detect(&vec![0.0; 22_050]).is_empty());
    }
}
