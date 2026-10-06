//! Unit tests of `encode.rs`: round trip through a decoder, the MD5 and hash, the size
//! guard on incompressible audio, determinism, and the refusals.

use std::io::Cursor;
use std::path::Path;

use md5::{Digest, Md5};
use sc_core::Error;

use super::*;
use crate::flac::test_build::{file, samples};
use crate::flac::{FlacPcm, StreamInfo, read_layout};

fn encode_all(data: &[i32], channels: u16, bits: u16, chunk: usize) -> (Vec<u8>, Encoded) {
    let mut enc = FrameEncoder::new(48_000, channels, bits, Path::new("o.flac")).expect("new");
    let mut out = Vec::new();
    for c in data.chunks(chunk * usize::from(channels)) {
        enc.push(c, &mut out).expect("push");
    }
    let done = enc.finish(&mut out).expect("finish");
    (out, done)
}

fn decode(frames: &[u8], done: &Encoded, channels: u16, bits: u16) -> Vec<i32> {
    let info = StreamInfo {
        min_block: 4096,
        max_block: 4096,
        min_frame: done.min_frame,
        max_frame: done.max_frame,
        sample_rate_hz: 48_000,
        channels: u8::try_from(channels).expect("small"),
        bits: u8::try_from(bits).expect("small"),
        total_samples: done.total_samples,
        md5: done.md5,
    };
    let bytes = file(&[], &info, &[], frames, &[]);
    let layout = read_layout(&mut Cursor::new(&bytes), Path::new("o.flac")).expect("walks");
    let mut pcm = FlacPcm::open(Cursor::new(&bytes), Path::new("o.flac"), &layout, true).expect("open");
    let (mut all, mut block) = (Vec::new(), Vec::new());
    while pcm.next_block(&mut block).expect("decodes") > 0 {
        all.extend_from_slice(&block);
    }
    pcm.finish().expect("count and MD5 match");
    all
}

#[test]
fn frames_decode_to_the_samples_with_matching_md5_and_hash() {
    for (channels, bits) in [(2, 24), (1, 16), (2, 16)] {
        let data = samples(10_000, channels, u8::try_from(bits).expect("small"), 5);
        let (frames, done) = encode_all(&data, channels, bits, 777);
        assert_eq!(done.total_samples, 10_000);
        assert_eq!(done.frame_offsets.len(), 3);
        assert_eq!(done.frame_offsets[0], 0);
        assert_eq!(done.frames_bytes, frames.len() as u64);
        let mut bytes = Vec::new();
        crate::flac::le_sample_bytes(&data, usize::from(bits / 8), &mut bytes);
        assert_eq!(done.md5, <[u8; 16]>::from(Md5::digest(&bytes)));
        assert_eq!(done.pcm_hash, *blake3::hash(&bytes).as_bytes());
        assert_eq!(decode(&frames, &done, channels, bits), data);
    }
}

#[test]
fn full_scale_noise_stays_within_the_verbatim_bound_and_is_deterministic() {
    let mut x = 0x1234_5678_u32;
    let data: Vec<i32> = (0..2 * 9000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x.cast_signed() >> 8
        })
        .collect();
    let (a, done) = encode_all(&data, 2, 24, 4096);
    let (b, _) = encode_all(&data, 2, 24, 1);
    assert_eq!(a, b, "the same samples give the same bytes, whatever the push sizes");
    assert!(done.max_frame as usize <= max_frame_bytes(2, 4096, 24));
    assert!(done.max_frame as usize > 4096 * 6, "noise does not compress");
    assert_eq!(decode(&a, &done, 2, 24), data);
}

#[test]
fn the_bound_counts_header_subframe_headers_alignment_and_crc() {
    assert_eq!(max_frame_bytes(2, 4096, 24), 16 + 2 * (1 + 4096 * 3) + 3);
    assert_eq!(max_frame_bytes(1, 10, 16), 16 + 21 + 3);
}

#[test]
fn bad_input_is_refused() {
    let p = Path::new("o.flac");
    assert!(matches!(FrameEncoder::new(44_100, 3, 16, p), Err(Error::InvalidArgument(_))));
    assert!(matches!(FrameEncoder::new(44_100, 2, 20, p), Err(Error::InvalidArgument(_))));
    let mut out = Vec::new();
    let mut enc = FrameEncoder::new(44_100, 2, 16, p).expect("new");
    assert!(matches!(enc.push(&[1, 2, 3], &mut out), Err(Error::Internal(_))));
    assert!(matches!(enc.push(&[32_768, 0], &mut out), Err(Error::Internal(_))));
    assert!(matches!(enc.push(&[0, -32_769], &mut out), Err(Error::Internal(_))));
    enc.push(&[-32_768, 32_767], &mut out).expect("the extremes are in range");
    let empty = FrameEncoder::new(44_100, 1, 24, p).expect("new");
    assert!(matches!(empty.finish(&mut out), Err(Error::Internal(_))));
}

#[test]
fn write_errors_name_the_output() {
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut enc = FrameEncoder::new(44_100, 1, 16, Path::new("out.flac")).expect("new");
    let err = enc.push(&vec![0; 4096], &mut Full).expect_err("fails");
    assert!(matches!(err, Error::Io { path, .. } if path == Path::new("out.flac")));
}
