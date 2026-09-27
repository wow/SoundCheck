//! Small numeric helpers shared by the grid's steps.

/// Centred running median over `window` values (shorter at the edges).
pub(super) fn running_median(values: &[f64], window: usize) -> Vec<f64> {
    let half = window / 2;
    (0..values.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half).min(values.len());
            let mut w = values[lo..hi.max(lo + 1)].to_vec();
            median(&mut w)
        })
        .collect()
}

pub(super) fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        values[n / 2]
    } else {
        f64::midpoint(values[n / 2 - 1], values[n / 2])
    }
}

/// Counts in this module are far below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
pub(super) fn count_f64(n: usize) -> f64 {
    n as f64
}

/// Grid indices in this module are far below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
pub(super) fn index_f64(i: i64) -> f64 {
    i as f64
}

/// `k mod bar` for an integer-valued `k`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn rem_index(k: f64, bar: usize) -> usize {
    k.rem_euclid(count_f64(bar)) as usize % bar
}

/// A frame index when `frame` is a non-negative integer value.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn frame_index(frame: f64) -> Option<usize> {
    (0.0..1e12).contains(&frame).then_some(frame as usize)
}

/// A shift in beats from `best` to `r`, in `(-bar/2, bar/2]`.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
pub(super) fn signed_shift(r: usize, best: usize, bar: usize) -> i8 {
    let d = (r + bar - best) % bar;
    let signed = if d * 2 > bar {
        d as i64 - bar as i64
    } else {
        d as i64
    };
    signed.clamp(-128, 127) as i8
}

/// A grid line index (an integer value far below 2^52) as an integer.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn line_index(j: f64) -> i64 {
    j as i64
}

/// A non-negative line offset as a slot of the residual lane.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn line_slot(offset: f64) -> Option<usize> {
    (offset >= 0.0).then_some(offset as usize)
}

/// A line count from a positive integer value.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn line_count(n: f64) -> usize {
    n.max(0.0) as usize
}

/// Reported statistics are display values; f32 keeps every meaningful digit.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn to_f32(x: f64) -> f32 {
    x as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_shifts_wrap_around_the_bar() {
        assert_eq!(signed_shift(1, 0, 4), 1);
        assert_eq!(signed_shift(2, 0, 4), 2);
        assert_eq!(signed_shift(3, 0, 4), -1);
        assert_eq!(signed_shift(0, 3, 4), 1);
        assert_eq!(signed_shift(4, 0, 9), 4);
        assert_eq!(signed_shift(5, 0, 9), -4);
    }
}
