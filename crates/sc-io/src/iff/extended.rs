//! The AIFF `COMM` sample rate: an 80-bit IEEE 754 extended value (Apple AIFF 1.3, "Extended"
//! type; IEEE 754-1985 double-extended with an explicit integer bit), decoded exactly as an
//! integer in hertz, and the exact encoding of one for the writers.

use std::fmt;

/// Why an 80-bit extended value is not a usable integer sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtendedRateError {
    /// Infinity or `NaN` (exponent all ones).
    NotFinite,
    /// Zero or negative.
    NotPositive,
    /// Not a whole number of hertz; the approximate value is given.
    NotInteger(f64),
    /// A whole number above `u32::MAX` Hz.
    TooLarge,
}

impl fmt::Display for ExtendedRateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFinite => write!(f, "sample rate is infinite or NaN"),
            Self::NotPositive => write!(f, "sample rate is zero or negative"),
            Self::NotInteger(hz) => write!(f, "sample rate {hz:.4} Hz is not a whole number"),
            Self::TooLarge => write!(f, "sample rate is above 4294967295 Hz"),
        }
    }
}

/// Exponent bias of the 80-bit format.
const BIAS: i32 = 16_383;

/// Decodes an 80-bit extended value (big-endian: sign bit, 15-bit exponent, 64-bit mantissa
/// whose top bit is the integer bit) that must hold a whole number of hertz, exactly.
///
/// # Errors
/// An [`ExtendedRateError`] for infinities, `NaN`, zero, negative, fractional or too large
/// values. Range checks for plausible rates are the caller's.
pub fn sample_rate_from_extended(bytes: [u8; 10]) -> Result<u32, ExtendedRateError> {
    let sign_exponent = u16::from_be_bytes([bytes[0], bytes[1]]);
    let mut m = [0_u8; 8];
    m.copy_from_slice(&bytes[2..]);
    let mantissa = u64::from_be_bytes(m);
    let exponent = sign_exponent & 0x7FFF;
    if exponent == 0x7FFF {
        return Err(ExtendedRateError::NotFinite);
    }
    if mantissa == 0 || sign_exponent & 0x8000 != 0 {
        return Err(ExtendedRateError::NotPositive);
    }
    // value = mantissa * 2^(e - 63)
    let e = i32::from(exponent) - BIAS;
    if e > 63 {
        return Err(ExtendedRateError::TooLarge);
    }
    if e < 0 {
        return Err(ExtendedRateError::NotInteger(approx(mantissa, e)));
    }
    let shift = 63 - e.unsigned_abs();
    let fraction = mantissa & ((1_u64 << shift) - 1);
    if fraction != 0 {
        return Err(ExtendedRateError::NotInteger(approx(mantissa, e)));
    }
    u32::try_from(mantissa >> shift).map_err(|_| ExtendedRateError::TooLarge)
}

/// Encodes a whole number of hertz as an 80-bit extended value, exactly: the inverse of
/// [`sample_rate_from_extended`]. The mantissa is normalised (integer bit set), as Apple's and
/// libsndfile's writers store it; 0 Hz encodes as positive zero.
#[must_use]
pub fn extended_from_sample_rate(hz: u32) -> [u8; 10] {
    let mut out = [0_u8; 10];
    if hz == 0 {
        return out;
    }
    let e = hz.ilog2();
    // e < 32, so the exponent is far inside 15 bits.
    let exponent = u16::try_from(BIAS.unsigned_abs() + e).unwrap_or(u16::MAX);
    let mantissa = u64::from(hz) << (63 - e);
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// The value as `f64`, for messages only.
fn approx(mantissa: u64, e: i32) -> f64 {
    // Precision loss is irrelevant in an error message.
    #[allow(clippy::cast_precision_loss)]
    let m = mantissa as f64;
    m * 2_f64.powi(e - 63)
}

#[cfg(test)]
mod tests;
