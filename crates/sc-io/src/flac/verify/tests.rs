//! Unit tests of `verify.rs`: symphonia decodes the frames to the same samples, MD5 and hash
//! as the source, reads them by range past leading and trailing bytes, and fails on damage.

#![allow(clippy::cast_possible_truncation)] // test sizes and offsets are far below 2^32

use std::path::Path;
use std::sync::atomic::AtomicBool;

use md5::{Digest, Md5};
use sc_core::Error;

use super::*;
use crate::flac::test_build::{encode, file, id3v1, id3v2, samples, vorbis};

fn write(dir: &Path, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join("v.flac");
    std::fs::write(&p, bytes).expect("write");
    p
}

#[test]
fn decodes_the_frames_by_range() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (channels, bits) in [(2_u16, 24_u8), (1, 16)] {
        let data = samples(9000, channels, bits, 17);
        let (frames, info) = encode(&data, 44_100, channels, bits);
        let blocks = [(4, vorbis("v", &["A=1"])), (6, vec![0xEE; 40])];
        let lead = id3v2(30);
        let bytes = file(&lead, &info, &blocks, &frames, &id3v1());
        let path = write(dir.path(), &bytes);
        let start = (bytes.len() - 128 - frames.len()) as u64;
        let range = start..start + frames.len() as u64;
        let got = decode_frames(
            &path,
            &info.to_bytes(),
            range,
            channels,
            u16::from(bits),
            &AtomicBool::new(false),
        )
        .expect("decodes");
        let mut le = Vec::new();
        crate::flac::le_sample_bytes(&data, usize::from(bits / 8), &mut le);
        assert_eq!(got.frames, 9000);
        assert_eq!(<[u8; 16]>::from(Md5::digest(&le)), info.md5);
        assert_eq!(got.pcm_hash, *blake3::hash(&le).as_bytes());
    }
}

#[test]
fn damage_changes_the_result_or_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = samples(9000, 2, 16, 19);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    let mut bytes = file(&[], &info, &[], &frames, &[]);
    let start = (bytes.len() - frames.len()) as u64;
    let mid = bytes.len() - frames.len() / 2;
    bytes[mid] ^= 0x04;
    let path = write(dir.path(), &bytes);
    let got = decode_frames(
        &path,
        &info.to_bytes(),
        start..bytes.len() as u64,
        2,
        16,
        &AtomicBool::new(false),
    );
    match got {
        Err(Error::Corrupt { path: p, .. }) => assert_eq!(p, path),
        Ok(d) => {
            let mut le = Vec::new();
            crate::flac::le_sample_bytes(&data, 2, &mut le);
            assert_ne!(
                d.pcm_hash,
                *blake3::hash(&le).as_bytes(),
                "damage must show"
            );
        }
        Err(e) => panic!("unexpected {e}"),
    }
    let cancelled = decode_frames(
        &path,
        &info.to_bytes(),
        start..bytes.len() as u64,
        2,
        16,
        &AtomicBool::new(true),
    );
    assert!(matches!(cancelled, Err(Error::Cancelled)));
}
