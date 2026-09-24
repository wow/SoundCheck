//! Unit-carrying newtypes. A bare `f32` level never crosses a crate boundary.
//!
//! Loudness units follow ITU-R BS.1770-5 and EBU R 128: LUFS is absolute loudness, LU is a
//! difference, dBTP is true-peak level. Sample positions are integer indices at the file's native
//! rate; seconds are derived from them, never the other way round.

use std::fmt;
use std::ops::Sub;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Floor used when converting a non-positive linear value to decibels.
pub const MIN_DBFS: f64 = -150.0;

macro_rules! unit_f64 {
    ($(#[$meta:meta])* $name:ident, $suffix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize, TS)]
        #[ts(export)]
        pub struct $name(pub f64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{:.2} {}", self.0, $suffix)
            }
        }
    };
}

unit_f64!(
    /// Absolute loudness in LUFS (ITU-R BS.1770-5).
    Lufs, "LUFS"
);
unit_f64!(
    /// A loudness difference in LU (1 LU = 1 dB).
    Lu, "LU"
);
unit_f64!(
    /// True-peak level in dBTP (BS.1770-5 Annex 2, 4x oversampled).
    DbTp, "dBTP"
);
unit_f64!(
    /// A sample-peak level or a gain in dB relative to full scale.
    DbFs, "dB"
);
unit_f64!(
    /// Tempo in beats per minute, carried to two decimals.
    Bpm, "BPM"
);
unit_f64!(
    /// A duration or position in seconds. Derived from samples; never the source of truth.
    Seconds, "s"
);

impl Sub for Lufs {
    type Output = Lu;

    fn sub(self, rhs: Self) -> Lu {
        Lu(self.0 - rhs.0)
    }
}

impl DbFs {
    /// Converts a linear amplitude to decibels; non-positive input clamps to [`MIN_DBFS`].
    #[must_use]
    pub fn from_linear(linear: f64) -> Self {
        if linear <= 0.0 {
            Self(MIN_DBFS)
        } else {
            Self((20.0 * linear.log10()).max(MIN_DBFS))
        }
    }

    /// Converts decibels to a linear amplitude factor.
    #[must_use]
    pub fn to_linear(self) -> f64 {
        10f64.powf(self.0 / 20.0)
    }
}

/// A position in samples (frames) at the file's native sample rate. The source of truth for every
/// beat, grid and cue position.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize, TS,
)]
#[ts(export)]
pub struct SampleIndex(#[ts(type = "number")] pub u64);

impl SampleIndex {
    /// Position in seconds at `sample_rate`.
    #[must_use]
    // Precision loss above 2^53 samples (about 60 000 hours at 44.1 kHz) is irrelevant here.
    #[allow(clippy::cast_precision_loss)]
    pub fn to_seconds(self, sample_rate: u32) -> Seconds {
        Seconds(self.0 as f64 / f64::from(sample_rate))
    }
}

impl Seconds {
    /// Nearest sample index at `sample_rate`; negative values clamp to zero.
    #[must_use]
    // The value is rounded and clamped to be non-negative before the cast, so truncation and
    // sign loss cannot occur.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn to_sample_index(self, sample_rate: u32) -> SampleIndex {
        let samples = (self.0 * f64::from(sample_rate)).round().max(0.0);
        SampleIndex(samples as u64)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact values are intended in these tests
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn dbfs_round_trips_through_linear() {
        assert_abs_diff_eq!(DbFs::from_linear(0.5).0, -6.020_6, epsilon = 1e-4);
        assert_abs_diff_eq!(DbFs(-6.0).to_linear(), 0.501_187, epsilon = 1e-6);
        assert_eq!(DbFs::from_linear(0.0), DbFs(MIN_DBFS));
    }

    #[test]
    fn samples_and_seconds_convert_both_ways() {
        let idx = SampleIndex(44_100);
        assert_eq!(idx.to_seconds(44_100), Seconds(1.0));
        assert_eq!(Seconds(0.5).to_sample_index(48_000), SampleIndex(24_000));
        assert_eq!(Seconds(-1.0).to_sample_index(48_000), SampleIndex(0));
    }

    #[test]
    fn loudness_difference_is_lu() {
        assert_eq!(Lufs(-8.0) - Lufs(-11.0), Lu(3.0));
        assert_eq!(format!("{}", Lufs(-11.0)), "-11.00 LUFS");
    }

    #[test]
    fn units_serialise_transparently() {
        assert_eq!(serde_json::to_string(&Bpm(128.0)).unwrap(), "128.0");
        assert_eq!(serde_json::to_string(&SampleIndex(7)).unwrap(), "7");
    }
}
