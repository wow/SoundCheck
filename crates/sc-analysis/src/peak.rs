//! Sample peak: the largest absolute sample value, in dBFS.

use sc_core::DbFs;

/// Largest absolute sample value over all channels, as dBFS. Silence reports the floor.
#[must_use]
pub fn sample_peak(data: &[f32]) -> DbFs {
    let peak = data.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
    DbFs::from_linear(f64::from(peak))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact values are intended in these tests
    use super::*;
    use approx::assert_abs_diff_eq;
    use sc_core::units::MIN_DBFS;

    #[test]
    fn half_scale_is_minus_six_db() {
        assert_abs_diff_eq!(sample_peak(&[0.1, -0.5, 0.2]).0, -6.020_6, epsilon = 1e-4);
        assert_eq!(sample_peak(&[0.0; 8]), DbFs(MIN_DBFS));
        assert_eq!(sample_peak(&[]), DbFs(MIN_DBFS));
    }
}
