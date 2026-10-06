//! Unit tests of `crates/sc-io/src/id3/parse.rs`: syncsafe integers, plain and syncsafe frame
//! sizes, extended headers, footers, and every reason a tag is not editable.

use super::*;
use crate::id3::test_build::{ext_v23, ext_v24_update, frame, tag, tag_with_footer, text, txxx};

#[test]
fn syncsafe_integers_round_trip_and_reject_high_bits() {
    for v in [0, 1, 127, 128, 200, 16_383, 16_384, (1 << 28) - 1] {
        let b = encode_syncsafe(v).expect("28 bits");
        assert!(b.iter().all(|x| x & 0x80 == 0), "{v}: {b:?}");
        assert_eq!(decode_syncsafe(b), Some(v));
    }
    assert_eq!(encode_syncsafe(200), Some([0, 0, 1, 0x48]));
    assert_eq!(encode_syncsafe(1 << 28), None);
    assert_eq!(decode_syncsafe([0, 0, 0x80, 0]), None);
}

#[test]
fn frame_sizes_are_plain_in_v23_and_syncsafe_in_v24() {
    let body = vec![0x41; 200];
    for major in [3, 4] {
        let f = frame(major, *b"PRIV", [0, 0], &body);
        let size = &f[4..8];
        if major == 3 {
            assert_eq!(size, [0, 0, 0, 200]);
        } else {
            assert_eq!(size, [0, 0, 1, 0x48]);
        }
        let t = tag(
            major,
            0,
            &[],
            &[f.clone(), text(major, *b"TBPM", 0, "128")],
            7,
        );
        let index = parse_tag(&t).expect("parses");
        assert_eq!(index.frames.len(), 2);
        assert_eq!(index.frames[0].range, 10..10 + f.len());
        assert_eq!(&index.frames[1].id, b"TBPM");
        assert_eq!(index.padding.len(), 7);
        assert_eq!(index.end, t.len());
    }
}

#[test]
fn a_v23_size_read_as_syncsafe_would_differ() {
    // 0x80 in a v2.3 frame size is a plain byte; v2.4 refuses it as not syncsafe.
    let f3 = frame(3, *b"PRIV", [0, 0], &[0; 128]);
    assert_eq!(&f3[4..8], [0, 0, 0, 0x80]);
    assert!(parse_tag(&tag(3, 0, &[], std::slice::from_ref(&f3), 0)).is_ok());
    let err = parse_tag(&tag(4, 0, &[], &[f3], 0)).expect_err("not syncsafe");
    assert!(matches!(err, NotEditable::Malformed { .. }), "{err}");
}

#[test]
fn extended_headers_are_indexed_by_version() {
    let frames = [text(3, *b"TIT2", 0, "A")];
    let t = tag(3, 0x40, &ext_v23(5), &frames, 5);
    let index = parse_tag(&t).expect("v2.3 with extended header");
    assert_eq!(index.extended, Some(10..20));
    assert_eq!(index.frames[0].range.start, 20);

    let frames = [text(4, *b"TIT2", 3, "A")];
    let t = tag(4, 0x40, &ext_v24_update(), &frames, 3);
    let index = parse_tag(&t).expect("v2.4 with extended header");
    assert_eq!(index.extended, Some(10..16));
    assert_eq!(index.padding.len(), 3);
}

#[test]
fn a_crc_restrictions_or_unsync_make_the_tag_not_editable() {
    let f3 = [text(3, *b"TIT2", 0, "A")];
    let crc23 = [&[0, 0, 0, 10, 0x80, 0, 0, 0, 0, 0][..], &[1, 2, 3, 4]].concat();
    assert_eq!(
        parse_tag(&tag(3, 0x40, &crc23, &f3, 0)),
        Err(NotEditable::ExtendedHeaderCrc)
    );
    let f4 = [text(4, *b"TIT2", 0, "A")];
    let crc24 = [0, 0, 0, 12, 1, 0x20, 5, 1, 2, 3, 4, 5];
    assert_eq!(
        parse_tag(&tag(4, 0x40, &crc24, &f4, 0)),
        Err(NotEditable::ExtendedHeaderCrc)
    );
    let restricted = [0, 0, 0, 8, 1, 0x10, 1, 0];
    assert_eq!(
        parse_tag(&tag(4, 0x40, &restricted, &f4, 0)),
        Err(NotEditable::Restricted)
    );
    for major in [3, 4] {
        assert_eq!(
            parse_tag(&tag(major, 0x80, &[], &f4, 0)),
            Err(NotEditable::Unsynchronised)
        );
    }
    assert_eq!(
        parse_tag(&tag(3, 0x10, &[], &f3, 0)),
        Err(NotEditable::UnknownFlags { flags: 0x10 })
    );
    assert_eq!(
        parse_tag(&tag(4, 0x01, &[], &f4, 0)),
        Err(NotEditable::UnknownFlags { flags: 0x01 })
    );
    // The experimental flag is defined in both versions.
    assert!(parse_tag(&tag(3, 0x20, &[], &f3, 0)).is_ok());
    assert_eq!(
        parse_tag(&tag(2, 0, &[], &[], 4)),
        Err(NotEditable::UnsupportedVersion { major: 2 })
    );
}

fn malformed(bytes: &[u8]) -> String {
    match parse_tag(bytes) {
        Err(NotEditable::Malformed { detail }) => detail,
        other => panic!("not malformed: {other:?}"),
    }
}

#[test]
fn structural_problems_are_malformed() {
    let good = tag(3, 0, &[], &[text(3, *b"TIT2", 0, "Title")], 4);
    assert!(malformed(&good[..12]).contains("declares"));
    assert!(malformed(b"ID").contains("header"));
    assert!(malformed(b"TAG\x03\x00\x00\x00\x00\x00\x00").contains("header"));

    let mut overrun = good.clone();
    overrun[17] = 40; // frame size past the tag
    assert!(malformed(&overrun).contains("runs past"));

    let mut bad_id = good.clone();
    bad_id[11] = b'i';
    assert!(malformed(&bad_id).contains("frame id"));

    let mut dirty = good.clone();
    let last = dirty.len() - 1;
    dirty[last] = 1;
    assert!(malformed(&dirty).contains("padding"));

    let mut size = good;
    size[9] = 0x80;
    assert!(malformed(&size).contains("syncsafe"));

    let mut footer = tag_with_footer(&[text(4, *b"TIT2", 3, "A")]);
    let at = footer.len() - 10;
    footer[at] = b'X';
    assert!(malformed(&footer).contains("3DI"));

    let short_ext = tag(3, 0x40, &[0, 0, 0, 2, 0, 0], &[], 0);
    assert!(malformed(&short_ext).contains("extended header size"));
}

#[test]
fn a_footer_ends_the_tag_and_bytes_after_it_are_not_part_of_it() {
    let mut t = tag_with_footer(&[text(4, *b"TIT2", 3, "A")]);
    let len = t.len();
    t.extend_from_slice(&[9, 9, 9]);
    let index = parse_tag(&t).expect("parses");
    assert!(index.footer);
    assert_eq!(index.end, len);
    assert!(index.padding.is_empty());
}

#[test]
fn txxx_descriptions_are_read_while_indexing() {
    let t = tag(
        4,
        0,
        &[],
        &[
            txxx(4, 1, "replaygain_track_gain", "-4.10 dB"),
            text(4, *b"TBPM", 0, "1"),
        ],
        0,
    );
    let index = parse_tag(&t).expect("parses");
    assert_eq!(
        index.frames[0].description.as_deref(),
        Some("replaygain_track_gain")
    );
    assert_eq!(index.frames[1].description, None);
}

#[test]
fn oversized_tags_and_too_many_frames_are_refused_without_reading_them() {
    let mut huge = tag(4, 0, &[], &[], 0);
    huge[6..10].copy_from_slice(&encode_syncsafe((1 << 28) - 1).expect("28 bits"));
    assert!(matches!(
        parse_tag(&huge),
        Err(NotEditable::TooLarge { .. })
    ));
    let empty = frame(3, *b"TXXX", [0, 0], &[]);
    let many: Vec<Vec<u8>> = vec![empty; MAX_FRAMES + 1];
    assert!(malformed(&tag(3, 0, &[], &many, 0)).contains("frames"));
}
