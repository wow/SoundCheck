//! Sample peak: the largest absolute sample value, in dBFS.

use sc_core::DbFs;

/// Largest absolute sample value over all channels, as dBFS. Silence reports the floor.
#[must_use]
pub fn sample_peak(data: &[f32]) -> DbFs {
    let peak = data.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
    DbFs::from_linear(f64::from(peak))
}

#[cfg(test)]
mod tests;
