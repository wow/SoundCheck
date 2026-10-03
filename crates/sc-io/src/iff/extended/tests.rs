use super::*;

/// Encodings as written by Apple's and libsndfile's AIFF writers.
const KNOWN: [(u32, [u8; 10]); 7] = [
    (8_000, [0x40, 0x0B, 0xFA, 0, 0, 0, 0, 0, 0, 0]),
    (22_050, [0x40, 0x0D, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
    (44_100, [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
    (48_000, [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]),
    (88_200, [0x40, 0x0F, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
    (96_000, [0x40, 0x0F, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]),
    (192_000, [0x40, 0x10, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]),
];

#[test]
fn common_rates_decode_exactly() {
    for (hz, bytes) in KNOWN {
        assert_eq!(sample_rate_from_extended(bytes), Ok(hz), "{hz} Hz");
        assert_eq!(super::super::test_build::extended(hz), bytes, "{hz} Hz");
    }
}

#[test]
fn every_integer_up_to_u32_max_round_trips() {
    for hz in [1, 2, 3, 1_000, 11_025, 352_800, 1_536_000, u32::MAX] {
        let bytes = super::super::test_build::extended(hz);
        assert_eq!(sample_rate_from_extended(bytes), Ok(hz), "{hz} Hz");
    }
}

#[test]
fn an_unnormalised_mantissa_still_decodes_exactly() {
    // 44100 with the integer bit clear: mantissa shifted right by one, exponent one higher.
    let bytes = [0x40, 0x0F, 0x56, 0x22, 0, 0, 0, 0, 0, 0];
    assert_eq!(sample_rate_from_extended(bytes), Ok(44_100));
}

#[test]
fn unusable_values_are_rejected() {
    let reject = |bytes: [u8; 10]| sample_rate_from_extended(bytes);
    // -44100
    assert_eq!(
        reject([0xC0, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::NotPositive)
    );
    // +0 and -0
    assert_eq!(reject([0; 10]), Err(ExtendedRateError::NotPositive));
    assert_eq!(
        reject([0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::NotPositive)
    );
    // +infinity and NaN
    assert_eq!(
        reject([0x7F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::NotFinite)
    );
    assert_eq!(
        reject([0x7F, 0xFF, 0xC0, 0, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::NotFinite)
    );
    // 44100.5 = 88201 / 2
    let half = reject([0x40, 0x0E, 0xAC, 0x44, 0x80, 0, 0, 0, 0, 0]);
    assert_eq!(half, Err(ExtendedRateError::NotInteger(44_100.5)));
    // 0.5
    assert_eq!(
        reject([0x3F, 0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::NotInteger(0.5))
    );
    // 2^40 and 2^100
    assert_eq!(
        reject([0x40, 0x27, 0x80, 0, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::TooLarge)
    );
    assert_eq!(
        reject([0x40, 0x63, 0x80, 0, 0, 0, 0, 0, 0, 0]),
        Err(ExtendedRateError::TooLarge)
    );
}

#[test]
fn the_classic_mac_rate_is_not_an_integer() {
    // 22254.5454... Hz (Macintosh 22 kHz), as old AIFF writers stored it.
    let bytes = [0x40, 0x0D, 0xAD, 0xDD, 0x17, 0x45, 0xD1, 0x74, 0x5D, 0x17];
    let Err(ExtendedRateError::NotInteger(hz)) = sample_rate_from_extended(bytes) else {
        panic!("a fractional rate must be rejected");
    };
    assert!((hz - 22_254.545_454).abs() < 1e-3, "{hz}");
}
