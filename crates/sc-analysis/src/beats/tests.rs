//! Unit tests of the private parts of `crates/sc-analysis/src/beats.rs`.
use super::*;

#[test]
fn expected_chunks_follow_beat_this_chunking() {
    // 1488 activation frames per step; frames = samples / 441 + 1.
    assert_eq!(expected_chunks(0), 1);
    assert_eq!(expected_chunks(441 * 1487), 1);
    assert_eq!(expected_chunks(441 * 1488), 2);
    // 30 s is 1501 frames: two chunks, as beat-this runs it.
    assert_eq!(expected_chunks(22_050 * 30), 2);
    assert_eq!(expected_chunks(22_050 * 240), 9);
}
