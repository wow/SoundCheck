use sc_core::Error;

use super::super::test_build::{Form, fmt_pcm};
use super::*;

fn ids(t: &ChunkTable) -> Vec<String> {
    t.chunks.iter().map(Chunk::id_text).collect()
}

#[test]
fn plain_riff_lists_chunks_with_exact_ranges() {
    let bytes = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &[1, 2, 3, 4, 5, 6, 7, 8])
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.container, Container::Riff);
    assert_eq!(t.form_size_declared, bytes.len() as u64 - 8);
    assert_eq!(t.file_len, bytes.len() as u64);
    assert_eq!(ids(&t), ["fmt ", "data"]);
    assert_eq!(t.chunks[0].header_offset, 12);
    assert_eq!(t.chunks[0].payload, 20..36);
    assert_eq!(t.chunks[1].header_offset, 36);
    assert_eq!(t.chunks[1].payload, 44..52);
    assert_eq!(t.chunks[1].end(), 52);
    assert!(t.chunks.iter().all(|c| c.pad.is_none() && !c.pad_missing));
    assert_eq!(t.trailing, None);
    assert!(!t.truncated);
}

#[test]
fn odd_chunks_keep_their_pad_byte_value() {
    let bytes = Form::riff()
        .chunk_pad(b"xodd", b"odd\0len", Some(0x20))
        .chunk(b"data", &[0; 4])
        .chunk_pad(b"last", b"abc", Some(0x00))
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["xodd", "data", "last"]);
    assert_eq!(t.chunks[0].pad, Some(0x20));
    assert_eq!(t.chunks[0].payload_len(), 7);
    assert_eq!(t.chunks[1].header_offset, 12 + 8 + 8);
    assert_eq!(t.chunks[2].pad, Some(0));
    assert_eq!(t.chunks[2].end(), bytes.len() as u64);
    assert_eq!(t.trailing, None);
}

#[test]
fn odd_last_data_without_pad_and_an_id3v1_tag_after_the_riff_end() {
    let mut tag = b"TAG".to_vec();
    tag.resize(128, b'x');
    let mut bytes = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 24))
        .chunk_pad(b"data", &[1, 2, 3, 4, 5, 6, 7, 8, 9], None)
        .build();
    let riff_end = bytes.len() as u64;
    bytes.extend_from_slice(&tag);
    let t = walk_bytes(&bytes).unwrap();
    let data = t.find(b"data").unwrap();
    assert_eq!(data.payload_len(), 9);
    assert_eq!(data.pad, None);
    assert!(data.pad_missing);
    assert_eq!(t.container_end(), riff_end);
    assert_eq!(t.trailing, Some(riff_end..bytes.len() as u64));
    assert!(!t.truncated);
}

#[test]
fn stray_bytes_after_the_container_are_trailing() {
    let mut bytes = Form::riff().chunk(b"data", &[0; 4]).build();
    bytes.extend_from_slice(b"\0\0\x01");
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["data"]);
    assert_eq!(t.trailing, Some(24..27));
}

#[test]
fn a_chunk_cut_by_the_end_of_the_file_is_clamped_and_flagged() {
    let mut f = Form::riff().chunk(b"fmt ", &fmt_pcm(2, 44_100, 16));
    f.chunk_sized(b"data", 1000, &[7; 10]);
    let bytes = f.build_with_size(4 + 24 + 8 + 1000);
    let t = walk_bytes(&bytes).unwrap();
    let data = t.find(b"data").unwrap();
    assert_eq!(data.size_declared, 1000);
    assert_eq!(data.payload, 44..54);
    assert!(data.is_clamped());
    assert!(t.truncated);
    assert_eq!(t.trailing, None);
}

#[test]
fn a_container_size_beyond_the_file_is_recorded_and_tolerated() {
    let bytes = Form::riff()
        .chunk(b"data", &[0; 4])
        .build_with_size(u32::MAX);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.form_size_declared, u64::from(u32::MAX));
    assert_eq!(ids(&t), ["data"]);
    assert!(!t.truncated);
    assert_eq!(t.trailing, None);
}

#[test]
fn a_chunk_running_past_a_short_container_size_is_kept_whole() {
    let f = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &[0; 8]);
    // The container size stops 4 bytes into the data payload.
    let bytes = f.build_with_size(4 + 24 + 8 + 4);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.find(b"data").unwrap().payload, 44..52);
    assert_eq!(t.trailing, None);
    assert!(!t.truncated);
}

#[test]
fn garbage_that_is_not_a_chunk_header_ends_the_walk() {
    let bytes = Form::riff()
        .chunk(b"data", &[0; 4])
        .raw(&[0; 12])
        .chunk(b"LIST", b"INFO")
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["data"]);
    assert_eq!(t.trailing, Some(24..bytes.len() as u64));
}

#[test]
fn a_missing_pad_between_chunks_is_detected_from_the_next_header() {
    let bytes = Form::riff()
        .chunk_pad(b"xodd", b"abc", None)
        .chunk(b"data", &[0; 4])
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["xodd", "data"]);
    assert_eq!(t.chunks[0].pad, None);
    assert!(t.chunks[0].pad_missing);
    assert_eq!(t.chunks[1].header_offset, 12 + 8 + 3);
}

#[test]
fn aiff_sizes_are_big_endian() {
    let bytes = Form::aiff()
        .chunk(b"COMM", &[0; 18])
        .chunk(b"NAME", b"odd")
        .chunk(b"SSND", &[0; 12])
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.container, Container::Aiff);
    assert_eq!(ids(&t), ["COMM", "NAME", "SSND"]);
    assert_eq!(t.chunks[1].pad, Some(0));
    assert_eq!(t.chunks[2].payload_len(), 12);
    let aifc = walk_bytes(&Form::aifc().build()).unwrap();
    assert_eq!(aifc.container, Container::Aifc);
    assert_eq!(aifc.chunks, []);
}

#[test]
fn non_iff_files_are_corrupt_and_other_forms_unsupported() {
    let corrupt = |b: &[u8]| matches!(walk_bytes(b), Err(Error::Corrupt { .. }));
    let unsupported = |b: &[u8]| matches!(walk_bytes(b), Err(Error::UnsupportedFormat { .. }));
    assert!(corrupt(b""));
    assert!(corrupt(b"RIFF\0\0\0\0WAV"));
    assert!(corrupt(b"fLaC\0\0\0\x22\0\0\0\0\0\0"));
    assert!(corrupt(b"ID3\x04\0\0\0\0\0\0\0\0\0\0"));
    assert!(unsupported(b"RIFF\x04\0\0\0AVI "));
    assert!(unsupported(b"RIFX\0\0\0\x04WAVE"));
    assert!(unsupported(b"FORM\0\0\0\x04MIDI"));
}

/// An RF64 file: `ds64` with the given table, a `bext` whose size lives in the table and a
/// `data` chunk whose size lives in `dataSize`.
fn rf64(table_count: u32, entries: &[([u8; 4], u64)]) -> Vec<u8> {
    let bext = [9_u8; 6];
    let data = [1_u8, 2, 3, 4, 5, 6];
    let mut ds64 = Vec::new();
    let ds64_len = 28 + 12 * entries.len();
    let riff_size = (4 + 8 + ds64_len + 8 + bext.len() + 8 + data.len()) as u64;
    ds64.extend_from_slice(&riff_size.to_le_bytes());
    ds64.extend_from_slice(&(data.len() as u64).to_le_bytes());
    ds64.extend_from_slice(&3_u64.to_le_bytes());
    ds64.extend_from_slice(&table_count.to_le_bytes());
    for (id, size) in entries {
        ds64.extend_from_slice(id);
        ds64.extend_from_slice(&size.to_le_bytes());
    }
    let mut out = b"RF64".to_vec();
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    for (id, size, payload) in [
        (b"ds64", u32::try_from(ds64.len()).unwrap(), ds64.as_slice()),
        (b"bext", u32::MAX, bext.as_slice()),
        (b"data", u32::MAX, data.as_slice()),
    ] {
        out.extend_from_slice(id);
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(payload);
    }
    out
}

#[test]
fn rf64_sizes_come_from_ds64() {
    let bytes = rf64(1, &[(*b"bext", 6)]);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.container, Container::Rf64);
    assert_eq!(t.form_size_declared, bytes.len() as u64 - 8);
    let ds64 = t.ds64.as_ref().unwrap();
    assert_eq!(
        (ds64.riff_size, ds64.data_size, ds64.sample_count),
        (bytes.len() as u64 - 8, 6, 3)
    );
    assert_eq!(
        ds64.table,
        [Ds64Entry {
            id: *b"bext",
            size: 6
        }]
    );
    assert_eq!(ids(&t), ["ds64", "bext", "data"]);
    assert_eq!(t.find(b"bext").unwrap().payload_len(), 6);
    let data = t.find(b"data").unwrap();
    assert_eq!(data.size_declared, 6);
    assert_eq!(data.payload.end, bytes.len() as u64);
    assert!(!t.truncated);
    assert_eq!(t.trailing, None);
}

#[test]
fn rf64_table_count_is_bounded_by_the_chunk() {
    // The table claims 4 billion entries; the chunk holds one.
    let bytes = rf64(u32::MAX, &[(*b"bext", 6)]);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.ds64.as_ref().unwrap().table.len(), 1);
    assert_eq!(ids(&t), ["ds64", "bext", "data"]);
}

#[test]
fn rf64_without_a_size_for_a_chunk_clamps_it() {
    // No table entry for `bext`: its 0xFFFFFFFF size runs past the end of the file.
    let bytes = rf64(0, &[]);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["ds64", "bext"]);
    assert!(t.truncated);
}

#[test]
fn rf64_without_ds64_is_corrupt() {
    let mut bytes = b"RF64\xFF\xFF\xFF\xFFWAVE".to_vec();
    bytes.extend_from_slice(b"fmt \x10\0\0\0");
    assert!(matches!(walk_bytes(&bytes), Err(Error::Corrupt { .. })));
    let mut short = b"RF64\xFF\xFF\xFF\xFFWAVEds64\x08\0\0\0".to_vec();
    short.extend_from_slice(&[0; 8]);
    assert!(matches!(walk_bytes(&short), Err(Error::Corrupt { .. })));
}
