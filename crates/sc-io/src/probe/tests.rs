//! Unit tests of the private parts of `crates/sc-io/src/probe.rs`.
use super::*;

#[test]
fn mp3_bitrates_snap_only_near_a_standard_rate() {
    assert_eq!(nominal_mp3_bitrate(129), 128);
    assert_eq!(nominal_mp3_bitrate(321), 320);
    assert_eq!(nominal_mp3_bitrate(245), 245);
    assert_eq!(nominal_mp3_bitrate(190), 192);
    assert_eq!(nominal_mp3_bitrate(185), 185);
}
