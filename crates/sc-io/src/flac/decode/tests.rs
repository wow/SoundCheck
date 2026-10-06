//! Unit tests of `decode.rs`: exact samples at several depths, where the frames end before a
//! trailing tag, the MD5 and count checks, corrupt and truncated frames, an unknown total.

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
    assert_eq!(pcm.delivered() as usize * usize::from(layout.streaminfo.channels), all.len());
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
    assert!(read_all(&bytes, false).is_ok(), "MD5 checked only on request");
    let unset = file(&[], &crate::flac::StreamInfo { md5: [0; 16], ..info }, &[], &frames, &[]);
    assert!(read_all(&unset, true).is_ok(), "no signature, nothing to check");
    let more = crate::flac::StreamInfo {
        total_samples: 5001,
        ..info
    };
    let bytes = file(&[], &more, &[], &frames, &[]);
    assert!(matches!(read_all(&bytes, false), Err(Error::Corrupt { .. })));
}

#[test]
fn damaged_and_truncated_frames_are_corrupt() {
    let data = samples(9000, 2, 16, 11);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    let good = file(&[], &info, &[], &frames, &[]);
    let mut damaged = good.clone();
    let mid = good.len() - frames.len() / 2;
    damaged[mid] ^= 0x10;
    assert!(matches!(read_all(&damaged, false), Err(Error::Corrupt { .. })));
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
    assert!(got == data);
    assert_eq!(read.frames_end, (bytes.len() - 128) as u64);
}
