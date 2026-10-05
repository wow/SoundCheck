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
    let data = t.find(b"data").unwrap();
    assert_eq!(data.payload, 44..52);
    assert!(data.beyond_container);
    assert!(!t.find(b"fmt ").unwrap().beyond_container);
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
    assert_eq!((data.size_field, data.size_declared), (u32::MAX, 6));
    assert_eq!(t.find(b"bext").unwrap().size_field, u32::MAX);
    assert_eq!(t.chunks[0].size_field, 28 + 12);
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

/// A 24-bit mono WAV whose odd `data` chunk (3 frames) has no pad byte, followed by `next`.
fn odd_data_then(next: &[u8], payload: &[u8]) -> Vec<u8> {
    Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 24))
        .chunk_pad(b"data", &[1, 2, 3, 4, 5, 6, 7, 8, 9], None)
        .chunk(next, payload)
        .build()
}

#[test]
fn a_missing_pad_before_list_is_not_taken_from_the_next_id() {
    // "LIST" with size 0x68: shifted by one byte the header reads "ISTh", a valid-looking id.
    let mut info = b"INFO".to_vec();
    info.resize(0x68, b'x');
    let bytes = odd_data_then(b"LIST", &info);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["fmt ", "data", "LIST"]);
    let data = t.find(b"data").unwrap();
    assert!(data.pad.is_none() && data.pad_missing);
    let list = t.find(b"LIST").unwrap();
    assert_eq!(
        (list.header_offset, list.payload_len()),
        (data.payload.end, 0x68)
    );
    assert!(!t.truncated);
    assert_eq!(t.trailing, None);
}

#[test]
fn a_missing_pad_before_an_id3_chunk_keeps_the_tag() {
    // "id3 " with size 0x2041: shifted by one byte the header reads "d3 A".
    let mut tag = b"ID3\x03\0\0".to_vec();
    tag.resize(0x2041, 0);
    let bytes = odd_data_then(b"id3 ", &tag);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["fmt ", "data", "id3 "]);
    assert!(t.find(b"data").unwrap().pad_missing);
    let id3 = t.find(b"id3 ").unwrap();
    assert_eq!((id3.payload_len(), id3.pad), (0x2041, Some(0)));
    assert!(!t.truncated);
    assert_eq!(t.trailing, None);
}

#[test]
fn a_printable_pad_before_a_valid_header_is_a_pad() {
    for pad in [b'Q', b' ', b'd', 0xFF] {
        let bytes = Form::riff()
            .chunk_pad(b"xodd", b"abc", Some(pad))
            .chunk(b"data", &[0; 4])
            .build();
        let t = walk_bytes(&bytes).unwrap();
        assert_eq!(ids(&t), ["xodd", "data"], "pad {pad:#x}");
        assert_eq!(t.chunks[0].pad, Some(pad));
        assert_eq!(t.chunks[1].header_offset, 12 + 8 + 4);
    }
}

#[test]
fn a_missing_pad_in_aiff_is_found_too() {
    let bytes = Form::aiff()
        .chunk_pad(b"NAME", b"odd", None)
        .chunk(b"ANNO", b"even")
        .chunk_pad(b"AUTH", b"abc", None)
        .chunk(b"SSND", &[0; 8])
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["NAME", "ANNO", "AUTH", "SSND"]);
    assert!(t.chunks[0].pad_missing && t.chunks[2].pad_missing);
    assert_eq!(t.trailing, None);
}

#[test]
fn a_container_size_of_zero_still_yields_the_chunks() {
    let bytes = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &[0; 8])
        .build_with_size(0);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.form_size_declared, 0);
    assert_eq!(ids(&t), ["fmt ", "data"]);
    assert!(t.chunks.iter().all(|c| c.beyond_container));
    assert_eq!(t.trailing, None);
    assert!(!t.truncated);
}

#[test]
fn an_id3_chunk_appended_after_a_stale_riff_size_is_walked() {
    let mut bytes = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &[0; 8])
        .build();
    let riff_end = bytes.len() as u64;
    bytes.extend_from_slice(b"id3 \x0B\0\0\0ID3\x03\0\0\0\0\0\0x\0");
    let id3_end = bytes.len() as u64;
    let mut v1 = b"TAG".to_vec();
    v1.resize(128, b'a');
    bytes.extend_from_slice(&v1);
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.container_end(), riff_end);
    assert_eq!(ids(&t), ["fmt ", "data", "id3 "]);
    let id3 = t.find(b"id3 ").unwrap();
    assert!(id3.beyond_container && !t.chunks[1].beyond_container);
    assert_eq!(
        (id3.payload_len(), id3.pad, id3.end()),
        (11, Some(0), id3_end)
    );
    assert_eq!(t.trailing, Some(id3_end..bytes.len() as u64));
}

#[test]
fn past_the_container_end_only_whole_chunks_count() {
    let mut bytes = Form::riff().chunk(b"data", &[0; 4]).build();
    let riff_end = bytes.len() as u64;
    bytes.extend_from_slice(b"JUNK\xE8\x03\0\0 only ten");
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["data"]);
    assert!(!t.truncated);
    assert_eq!(t.trailing, Some(riff_end..bytes.len() as u64));
}

#[test]
fn more_than_65536_chunks_is_corrupt() {
    let empty = b"JUNK\0\0\0\0";
    let many = |n: usize| Form::riff().raw(&empty.repeat(n)).build();
    assert_eq!(
        walk_bytes(&many(MAX_CHUNKS)).unwrap().chunks.len(),
        MAX_CHUNKS
    );
    assert!(matches!(
        walk_bytes(&many(MAX_CHUNKS + 1)),
        Err(Error::Corrupt { .. })
    ));
}

#[test]
fn more_than_65536_ds64_entries_is_corrupt() {
    let n = usize::try_from(MAX_DS64_ENTRIES).unwrap();
    let entries = vec![(*b"bext", 6_u64); n + 1];
    let count = u32::try_from(n + 1).unwrap();
    assert!(matches!(
        walk_bytes(&rf64(count, &entries)),
        Err(Error::Corrupt { .. })
    ));
    let t = walk_bytes(&rf64(count - 1, &entries[..n])).unwrap();
    assert_eq!(t.ds64.unwrap().table.len(), n);
}

/// An `ID3v1` tag with `title` (zero-filled to 30 bytes) and everything else zero.
fn id3v1(title: &[u8]) -> Vec<u8> {
    let mut tag = b"TAG".to_vec();
    tag.extend_from_slice(title);
    tag.resize(128, 0);
    tag
}

#[derive(Debug, Clone, Copy)]
enum RiffSize {
    Exact,
    IncludingTag,
    EightTooLarge,
    Zero,
}

#[test]
fn an_id3v1_tag_after_odd_data_stays_trailing_whatever_the_riff_size() {
    let titles: [&[u8]; 5] = [b"A", b"Go", b"Ax", b"A full printable title, 30 ch", b""];
    for size in [
        RiffSize::Exact,
        RiffSize::IncludingTag,
        RiffSize::EightTooLarge,
        RiffSize::Zero,
    ] {
        for padded in [false, true] {
            for title in titles {
                let case = format!("{size:?}, padded {padded}, title {title:?}");
                let form = Form::riff()
                    .chunk(b"fmt ", &fmt_pcm(1, 44_100, 24))
                    .chunk_pad(b"data", &[1, 2, 3, 4, 5, 6, 7, 8, 9], padded.then_some(0));
                let body = u32::try_from(4 + form.body.len()).unwrap();
                let field = match size {
                    RiffSize::Exact => body,
                    RiffSize::IncludingTag => body + 128,
                    RiffSize::EightTooLarge => body + 8,
                    RiffSize::Zero => 0,
                };
                let mut bytes = form.build_with_size(field);
                let tag = id3v1(title);
                bytes.extend_from_slice(&tag);
                let len = bytes.len() as u64;
                let t = walk_bytes(&bytes).unwrap();
                assert_eq!(ids(&t), ["fmt ", "data"], "{case}");
                let data = t.find(b"data").unwrap();
                assert_eq!(data.payload_len(), 9, "{case}");
                assert_eq!(data.pad, padded.then_some(0), "{case}");
                assert!(!t.truncated, "{case}");
                assert_eq!(t.trailing, Some(len - 128..len), "{case}");
                assert_eq!(&bytes[bytes.len() - 128..], tag.as_slice(), "{case}");
            }
        }
    }
}

#[test]
fn data_that_merely_starts_with_tag_128_bytes_from_the_end_is_kept() {
    let mut audio = vec![7_u8; 200];
    audio[200 - 128..200 - 125].copy_from_slice(b"TAG");
    let bytes = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &audio)
        .build();
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(t.find(b"data").unwrap().payload_len(), 200);
    assert_eq!(t.trailing, None);
    assert!(!t.truncated);
}

#[test]
fn a_zero_pad_left_out_of_the_container_size_is_a_pad() {
    let form = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk_pad(b"xodd", b"abc", Some(0));
    // The RIFF size stops before the pad byte; an `id3 ` chunk follows outside the container.
    let mut bytes = form.build_with_size(u32::try_from(4 + form.body.len() - 1).unwrap());
    bytes.extend_from_slice(b"id3 \x04\0\0\0ID3\x04");
    let t = walk_bytes(&bytes).unwrap();
    assert_eq!(ids(&t), ["fmt ", "xodd", "id3 "]);
    let odd = t.find(b"xodd").unwrap();
    assert_eq!(odd.pad, Some(0));
    assert!(odd.beyond_container);
    assert_eq!(t.find(b"id3 ").unwrap().payload_len(), 4);
    assert_eq!(t.trailing, None);
}

/// 65,536 one-byte chunks with a non-zero pad each: every pad decision reads a chain of
/// headers.
fn many_odd_chunks() -> Vec<u8> {
    Form::riff()
        .raw(&b"XXXX\x01\0\0\0ab".repeat(MAX_CHUNKS))
        .build()
}

#[test]
fn pad_probes_stay_within_the_walk_budget() {
    let bytes = many_odd_chunks();
    let (t, probes) = walk_with_budget(
        &mut std::io::Cursor::new(&bytes),
        Path::new(MEMORY_PATH),
        MAX_PAD_PROBES,
    )
    .unwrap();
    assert!(probes <= MAX_PAD_PROBES, "{probes} probes");
    assert_eq!(t.chunks.len(), MAX_CHUNKS);
    assert!(t.chunks.iter().all(|c| c.pad == Some(b'b')));
    // With a small budget the walk still finishes; later pads are assumed present.
    let (t, probes) = walk_with_budget(
        &mut std::io::Cursor::new(&bytes),
        Path::new(MEMORY_PATH),
        1_000,
    )
    .unwrap();
    assert!(probes <= 1_000, "{probes} probes");
    assert_eq!(t.chunks.len(), MAX_CHUNKS);
    assert!(t.chunks.iter().all(|c| c.pad == Some(b'b')));
    assert_eq!(t.trailing, None);
}
