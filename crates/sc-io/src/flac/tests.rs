//! Unit tests of `crates/sc-io/src/flac/mod.rs`: block headers, the MD5 byte layout, the
//! stream head fed to decoders.

use super::*;

#[test]
fn block_headers_pack_flag_type_and_length() {
    assert_eq!(block_header(false, 4, 0x01_02_03), [0x04, 0x01, 0x02, 0x03]);
    assert_eq!(block_header(true, 1, 10), [0x81, 0, 0, 10]);
    assert_eq!(block_header(true, 126, MAX_BLOCK_BYTES), [0xFE, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn sample_bytes_are_little_endian_and_sign_extended() {
    let mut out = vec![9; 3];
    le_sample_bytes(&[-2, 0x12_3456], 3, &mut out);
    assert_eq!(out, [0xFE, 0xFF, 0xFF, 0x56, 0x34, 0x12]);
    le_sample_bytes(&[-2, 0x1234], 2, &mut out);
    assert_eq!(out, [0xFE, 0xFF, 0x34, 0x12]);
}

#[test]
fn stream_head_is_marker_and_last_streaminfo() {
    let head = stream_head(&[7; STREAMINFO_BYTES]);
    assert_eq!(&head[..8], b"fLaC\x80\x00\x00\x22");
    assert_eq!(&head[8..], &[7; STREAMINFO_BYTES]);
}
