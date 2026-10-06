//! Unit tests of `walk.rs`: block listing, leading and trailing tags, and every refusal.

#![allow(clippy::cast_possible_truncation)] // test sizes and offsets are far below 2^32

use std::io::Cursor;
use std::path::Path;

use sc_core::Error;

use super::*;
use crate::flac::test_build::{encode, file, id3v1, id3v2, samples, vorbis};

fn walk(bytes: &[u8]) -> sc_core::Result<FlacLayout> {
    read_layout(&mut Cursor::new(bytes.to_vec()), Path::new("t.flac"))
}

fn stream(blocks: &[(u8, Vec<u8>)], leading: &[u8], trailing: &[u8]) -> (Vec<u8>, usize) {
    let data = samples(5000, 2, 16, 3);
    let (frames, info) = encode(&data, 44_100, 2, 16);
    (
        file(leading, &info, blocks, &frames, trailing),
        frames.len(),
    )
}

#[test]
fn lists_blocks_and_where_the_frames_start() {
    let blocks = [
        (4, vorbis("v", &["A=1"])),
        (100, vec![1, 2, 3]),
        (1, vec![0; 9]),
    ];
    let (bytes, frames) = stream(&blocks, &[], &[]);
    let l = walk(&bytes).expect("walks");
    let got: Vec<(u8, bool, u64)> = l
        .blocks
        .iter()
        .map(|b| (b.block_type, b.last, b.len()))
        .collect();
    assert_eq!(
        got,
        [
            (0, false, 34),
            (4, false, 16),
            (100, false, 3),
            (1, true, 9)
        ]
    );
    assert_eq!(l.blocks[2].name(), "RESERVED");
    assert_eq!(
        l.blocks[3].payload,
        l.blocks[3].header_offset + 4..l.frames_start
    );
    assert_eq!(l.marker_offset, 0);
    assert_eq!(l.frames_start as usize + frames, bytes.len());
    assert_eq!(
        (l.tail_tag_start, l.frames_limit()),
        (None, bytes.len() as u64)
    );
    assert_eq!(l.streaminfo.total_samples, 5000);
    assert_eq!(l.streaminfo_bytes, l.streaminfo.to_bytes());
}

#[test]
fn leading_id3v2_and_trailing_id3v1_and_ape_are_found() {
    let lead = [id3v2(20), id3v2(0)].concat();
    let mut ape = b"APETAGEX".to_vec();
    ape.extend_from_slice(&2000_u32.to_le_bytes()); // version
    ape.extend_from_slice(&(32_u32 + 10).to_le_bytes()); // items + footer
    ape.extend_from_slice(&0_u32.to_le_bytes()); // item count
    ape.extend_from_slice(&0x8000_0000_u32.to_le_bytes()); // header present
    ape.extend_from_slice(&[0; 8]);
    let tail = [vec![b'A'; 32], vec![b'I'; 10], ape, id3v1()].concat();
    let (bytes, frames) = stream(&[], &lead, &tail);
    let l = walk(&bytes).expect("walks");
    assert_eq!(l.marker_offset, lead.len() as u64);
    assert_eq!(l.leading(), 0..lead.len() as u64);
    assert_eq!(l.blocks.len(), 1);
    assert!(l.blocks[0].last);
    let frames_end = l.frames_start + frames as u64;
    assert_eq!(l.tail_tag_start, Some(frames_end));
    // Only an ID3v1 tag.
    let (bytes, frames) = stream(&[], &[], &id3v1());
    let l = walk(&bytes).expect("walks");
    assert_eq!(l.tail_tag_start, Some(l.frames_start + frames as u64));
}

fn refused(bytes: &[u8]) -> Error {
    walk(bytes).expect_err("refused")
}

#[test]
fn malformed_streams_are_refused() {
    let (good, _) = stream(&[(1, vec![0; 4])], &[], &[]);
    assert!(matches!(
        refused(b"RIFF....WAVE"),
        Error::UnsupportedFormat { .. }
    ));
    assert!(matches!(refused(&[]), Error::UnsupportedFormat { .. }));
    let mut bad_id3 = id3v2(4);
    bad_id3[9] = 0x80;
    bad_id3.extend_from_slice(&good);
    assert!(matches!(refused(&bad_id3), Error::Corrupt { .. }));
    // Every mutation below names the problem as a corrupt file.
    let mutate = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        b
    };
    let cases = [
        mutate(4, 0x01),  // the first block is PADDING
        mutate(7, 33),    // STREAMINFO of 33 bytes
        mutate(42, 0x7F), // forbidden type 127 (header of the PADDING block)
        mutate(42, 0x00), // a second STREAMINFO
        mutate(43, 0xFF), // a block running past the end of the file
        mutate(42, 0x01), // PADDING not last: the metadata runs on into frames
    ];
    for (i, case) in cases.iter().enumerate() {
        assert!(matches!(refused(case), Error::Corrupt { .. }), "case {i}");
    }
    assert!(matches!(refused(&good[..44]), Error::Corrupt { .. }));
}
