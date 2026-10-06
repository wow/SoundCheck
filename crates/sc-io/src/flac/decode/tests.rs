//! Unit tests of `decode.rs`: exact samples at several depths, where the frames end before a
//! trailing tag, the MD5 and count checks, corrupt and truncated frames, an unknown total.

#![allow(clippy::cast_possible_truncation)] // test sizes and offsets are far below 2^32

use std::io::Cursor;
use std::path::Path;

use sc_core::Error;

use super::*;
use crate::flac::read_layout;
use crate::flac::test_build::{encode, file, id3v1, samples};

fn read_all(bytes: &[u8], check_md5: bool) -> sc_core::Result<(Vec<i32>, FramesRead)> {
    let path = Path::new("s.flac");
    let layout = read_layout(&mut Cursor::new(bytes), path)?;
    let mut pcm = FlacPcm::open(Cursor::new(bytes), path, &layout, check_md5)?;
    let (mut all, mut block) = (Vec::new(), Vec::new());
    while pcm.next_block(&mut block)? > 0 {
        all.extend_from_slice(&block);
    }
    assert_eq!(
        pcm.delivered() as usize * usize::from(layout.streaminfo.channels),
        all.len()
    );
    let read = pcm.finish()?;
    Ok((all, read))
}

#[test]
fn samples_are_exact_and_the_frames_end_before_a_tail_tag() {
    for (channels, bits) in [(2, 16), (1, 24), (2, 20), (1, 8)] {
        let data = samples(9000, channels, bits, 7);
        let (frames, info) = encode(&data, 44_100, channels, bits);
        let bytes = file(&[], &info, &[(1, vec![0; 8])], &frames, &id3v1());
        let (got, read) = read_all(&bytes, true).expect("decodes");
        assert!(got == data, "{channels} ch {bits} bits");
        assert_eq!(read.frames, 9000);
        assert_eq!(read.frames_end, (bytes.len() - 128) as u64);
    }
}

#[test]
fn a_wrong_md5_or_count_is_corrupt() {
    let data = samples(5000, 2, 16, 9);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    let mut wrong = info;
    wrong.md5[0] ^= 1;
    let bytes = file(&[], &wrong, &[], &frames, &[]);
    assert!(matches!(read_all(&bytes, true), Err(Error::Corrupt { .. })));
    assert!(
        read_all(&bytes, false).is_ok(),
        "MD5 checked only on request"
    );
    let unset = file(
        &[],
        &crate::flac::StreamInfo {
            md5: [0; 16],
            ..info
        },
        &[],
        &frames,
        &[],
    );
    assert!(
        read_all(&unset, true).is_ok(),
        "no signature, nothing to check"
    );
    let more = crate::flac::StreamInfo {
        total_samples: 5001,
        ..info
    };
    let bytes = file(&[], &more, &[], &frames, &[]);
    assert!(matches!(
        read_all(&bytes, false),
        Err(Error::Corrupt { .. })
    ));
}

#[test]
fn damaged_and_truncated_frames_are_corrupt() {
    let data = samples(9000, 2, 16, 11);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    let good = file(&[], &info, &[], &frames, &[]);
    let mut damaged = good.clone();
    let mid = good.len() - frames.len() / 2;
    damaged[mid] ^= 0x10;
    assert!(matches!(
        read_all(&damaged, false),
        Err(Error::Corrupt { .. })
    ));
    let cut = &good[..good.len() - 100];
    assert!(matches!(read_all(cut, false), Err(Error::Corrupt { .. })));
}

#[test]
fn an_unknown_total_reads_to_the_tail_tag() {
    let data = samples(9000, 1, 16, 13);
    let (frames, info) = encode(&data, 48_000, 1, 16);
    let unknown = crate::flac::StreamInfo {
        total_samples: 0,
        ..info
    };
    let bytes = file(&[], &unknown, &[], &frames, &id3v1());
    let (got, read) = read_all(&bytes, true).expect("decodes");
    assert_eq!(got, data);
    assert_eq!(read.frames_end, (bytes.len() - 128) as u64);
}

/// Regression: a frame running past the declared total made the decoder count the samples
/// left below zero (a panic in debug builds); it is now a corrupt file.
#[test]
fn a_frame_past_the_declared_total_is_corrupt_not_a_panic() {
    let data = samples(9000, 2, 16, 43);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    let short = crate::flac::StreamInfo {
        total_samples: 5000,
        ..info
    };
    let bytes = file(&[], &short, &[], &frames, &[]);
    let err = read_all(&bytes, false).expect_err("refused");
    assert!(matches!(err, Error::Corrupt { .. }), "{err}");
}

#[test]
fn a_total_on_a_frame_boundary_stops_there_and_more_frames_follow() {
    let data = samples(9000, 2, 16, 45);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    let short = crate::flac::StreamInfo {
        total_samples: 8192,
        ..info
    };
    let bytes = file(&[], &short, &[], &frames, &id3v1());
    let (got, read) = read_all(&bytes, false).expect("decodes the declared samples");
    assert_eq!(got, data[..8192 * 2]);
    let path = Path::new("s.flac");
    let len = bytes.len() as u64;
    let mut src = Cursor::new(&bytes);
    assert!(frame_follows(&mut src, path, read.frames_end, len).expect("reads"));
    // At the ID3v1 tag and at the end of the file no frame follows.
    assert!(!frame_follows(&mut src, path, len - 128, len).expect("reads"));
    assert!(!frame_follows(&mut src, path, len, len).expect("reads"));
}

#[test]
fn frame_headers_need_sync_codes_and_a_matching_crc() {
    let data = samples(5000, 1, 24, 47);
    let (frames, _) = encode(&data, 48_000, 1, 24);
    assert!(is_frame_header(&frames[..16]));
    let mut bad_crc = frames[..16].to_vec();
    // The CRC-8 of a header with a one-byte frame number and no extra fields is byte 5.
    bad_crc[5] ^= 1;
    assert!(!is_frame_header(&bad_crc));
    let mut reserved = frames[..16].to_vec();
    reserved[2] |= 0x0F; // sample rate code 15 is invalid
    assert!(!is_frame_header(&reserved));
    assert!(!is_frame_header(b"TAGxxxxxxxxxxxxx"));
    assert!(!is_frame_header(&[0xFF, 0xF8]));
    assert!(!is_frame_header(&[]));
}
