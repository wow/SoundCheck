//! Unit tests of `streaminfo.rs`: the bit packing against hand-written bytes.

use super::*;

#[test]
fn packing_round_trips_and_matches_literal_bytes() {
    let info = StreamInfo {
        min_block: 4096,
        max_block: 4096,
        min_frame: 14,
        max_frame: 0x01_2345,
        sample_rate_hz: 44_100,
        channels: 2,
        bits: 24,
        total_samples: 19_559,
        md5: [0xAB; 16],
    };
    let bytes = info.to_bytes();
    // 44100 = 0x0AC44: rate 20 bits, then channels-1 = 1 (3 bits), bits-1 = 23 (5 bits),
    // total 36 bits: 0x0AC44 << 44 | 1 << 41 | 23 << 36 | 19559.
    let packed: u64 = 0x0AC44 << 44 | 1 << 41 | 23 << 36 | 0x4C67; // 19,559
    let mut want = vec![0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x0E, 0x01, 0x23, 0x45];
    want.extend_from_slice(&packed.to_be_bytes());
    want.extend_from_slice(&[0xAB; 16]);
    assert_eq!(bytes.to_vec(), want);
    assert_eq!(StreamInfo::parse(&bytes), info);
    assert!(info.has_md5());
    assert_eq!(info.md5_bytes_per_sample(), 3);
}

#[test]
fn extremes_survive_the_bit_widths() {
    let info = StreamInfo {
        min_block: 16,
        max_block: 65_535,
        min_frame: 0,
        max_frame: (1 << 24) - 1,
        sample_rate_hz: (1 << 20) - 1,
        channels: 8,
        bits: 32,
        total_samples: MAX_TOTAL_SAMPLES,
        md5: [0; 16],
    };
    assert_eq!(StreamInfo::parse(&info.to_bytes()), info);
    assert!(!info.has_md5());
    let odd = StreamInfo { bits: 20, ..info };
    assert_eq!(odd.md5_bytes_per_sample(), 3);
}
