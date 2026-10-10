//! Unit tests of `crates/sc-io/src/id3/parse.rs`: syncsafe integers, plain and syncsafe frame
//! sizes, extended headers, footers, and every reason a tag is not editable.

use super::*;
use crate::id3::test_build::{
    comm, comm_body, ext_v23, ext_v24_update, frame, geob, geob_body, tag, tag_with_footer, text,
    txxx, unsynchronise,
};

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
    for byte in 3..10 {
        let mut footer = tag_with_footer(&[text(4, *b"TIT2", 3, "A")]);
        let at = footer.len() - 10 + byte;
        footer[at] ^= 0x01;
        assert!(malformed(&footer).contains("repeat"), "footer byte {byte}");
    }

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

/// A v2.4 frame whose size field holds `size` as a plain integer, as old iTunes versions wrote.
fn plain_sized(id: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut f = frame(4, id, [0, 0], body);
    f[4..8].copy_from_slice(&u32::try_from(body.len()).expect("small").to_be_bytes());
    f
}

#[test]
fn regression_v24_plain_frame_size_with_a_zero_tail_is_ambiguous() {
    // A PRIV of 256 bytes stored with the plain size 0x100; its last 128 bytes are zeros. Read
    // as syncsafe the size is 128, and the zero tail would pass for padding an edit writes into.
    let mut body = b"com.example.itunes\0".to_vec();
    body.resize(128, 0x5A);
    body.resize(256, 0);
    let t = tag(
        4,
        0,
        &[],
        &[text(4, *b"TIT2", 0, "A"), plain_sized(*b"PRIV", &body)],
        0,
    );
    assert_eq!(&t[t.len() - 256 - 6..t.len() - 256 - 2], [0, 0, 1, 0]);
    assert!(malformed(&t).contains("ambiguous frame sizes"));
    // With padding after the full plain frame too.
    let padded = tag(4, 0, &[], &[plain_sized(*b"PRIV", &body)], 40);
    assert!(malformed(&padded).contains("ambiguous"));
    assert!(matches!(
        crate::id3::edit_tag(&t, &[crate::id3::Edit::new("TBPM", "1").expect("valid")]),
        Err(NotEditable::Malformed { .. })
    ));
}

#[test]
fn syncsafe_sizes_stand_where_the_plain_reading_does_not_fit() {
    let big = vec![0x41; 200];
    // Followed by a frame header: the syncsafe reading is trusted.
    let t = tag(
        4,
        0,
        &[],
        &[frame(4, *b"PRIV", [0, 0], &big), text(4, *b"TIT2", 0, "A")],
        600,
    );
    assert_eq!(parse_tag(&t).expect("parses").frames.len(), 2);
    // Last before padding too short for the plain reading.
    let t = tag(4, 0, &[], &[frame(4, *b"PRIV", [0, 0], &big)], 100);
    assert!(parse_tag(&t).is_ok());
    // Last before ample padding: refused for binary data, accepted for text.
    let t = tag(4, 0, &[], &[frame(4, *b"PRIV", [0, 0], &big)], 1024);
    assert!(malformed(&t).contains("ambiguous"));
    for id in [*b"TXXX", *b"COMM", *b"WXXX", *b"USLT"] {
        let t = tag(4, 0, &[], &[frame(4, id, [0, 0], &big)], 1024);
        assert!(parse_tag(&t).is_ok(), "{id:?}");
    }
    // v2.3 sizes are plain by definition.
    let t = tag(3, 0, &[], &[frame(3, *b"PRIV", [0, 0], &big)], 1024);
    assert!(parse_tag(&t).is_ok());
}

#[test]
fn geob_heads_are_read_in_every_encoding() {
    let object = [0x01, 0x01, 0xFF, 0x00, 0xE0];
    for major in [3, 4] {
        let utf8 = if major == 4 { 3 } else { 1 };
        let frames = [
            geob(major, "Serato Markers2", &object),
            frame(
                major,
                *b"GEOB",
                [0, 0],
                &geob_body(
                    1,
                    "application/x",
                    "Ünïcode.bin",
                    "Sérato Ovërview",
                    &object,
                ),
            ),
            frame(
                major,
                *b"GEOB",
                [0, 0],
                &geob_body(utf8, "text/plain", "", "Notes ♫", b"x"),
            ),
        ];
        let index = parse_tag(&tag(major, 0, &[], &frames, 16)).expect("parses");
        let f = &index.frames;
        assert_eq!(f[0].description.as_deref(), Some("Serato Markers2"));
        assert_eq!(f[0].mime.as_deref(), Some("application/octet-stream"));
        assert_eq!(f[0].file_name.as_deref(), Some(""));
        assert_eq!((f[0].encoding, f[0].language), (Some(0), None));
        assert!(f[0].is(b"GEOB", "serato markers2"));
        assert!(!f[0].is(b"TXXX", "Serato Markers2"));
        assert_eq!(f[1].description.as_deref(), Some("Sérato Ovërview"));
        assert_eq!(f[1].file_name.as_deref(), Some("Ünïcode.bin"));
        assert_eq!(f[1].mime.as_deref(), Some("application/x"));
        assert_eq!(
            (f[1].encoding, f[1].description_be.as_deref()),
            (Some(1), None)
        );
        assert_eq!(f[2].description.as_deref(), Some("Notes ♫"), "v2.{major}");
        assert_eq!(f[2].encoding, Some(utf8));
    }
}

#[test]
fn comm_heads_are_read_in_every_encoding() {
    let no_bom: Vec<u8> = [&[1][..], b"eng", b"i\0T\0\0\0", b"x\0"].concat();
    let frames = [
        comm(3, 0, "iTunNORM", " 00000A2B 00000A2B"),
        comm(3, 1, "Kommentar äöü", "text"),
        frame(3, *b"COMM", [0, 0], &no_bom),
        frame(
            3,
            *b"COMM",
            [0, 0],
            &comm_body(0, *b"deu", "", "plain comment"),
        ),
        frame(3, *b"COMM", [0, 0], b"\0en"),
    ];
    let index = parse_tag(&tag(3, 0, &[], &frames, 0)).expect("parses");
    let f = &index.frames;
    assert_eq!(f[0].description.as_deref(), Some("iTunNORM"));
    assert_eq!(f[0].language, Some(*b"eng"));
    assert!(f[0].is(b"COMM", "ITUNNORM"));
    assert_eq!(f[1].description.as_deref(), Some("Kommentar äöü"));
    assert_eq!(f[1].encoding, Some(1));
    // UTF-16 without a byte-order mark: little-endian first, big-endian as the alternative.
    assert_eq!(f[2].description.as_deref(), Some("iT"));
    assert_eq!(f[2].description_be.as_deref(), Some("\u{6900}\u{5400}"));
    assert_eq!(f[3].description.as_deref(), Some(""));
    assert_eq!(f[3].language, Some(*b"deu"));
    // Too short for a language: no head, the frame is still indexed.
    assert_eq!((f[4].description.as_ref(), f[4].language), (None, None));
    assert_eq!(f[4].mime, None);
    // UTF-8 in v2.4.
    let v24 = parse_tag(&tag(4, 0, &[], &[comm(4, 3, "Beschreibung ß", "x")], 0)).expect("v2.4");
    assert_eq!(v24.frames[0].description.as_deref(), Some("Beschreibung ß"));
}

#[test]
fn heads_are_read_behind_format_flags_and_unsynchronisation() {
    let body = geob_body(
        1,
        "application/octet-stream",
        "f",
        "Serato Autotags",
        &[0xFF, 0xE0],
    );
    // v2.4 frame-level unsynchronisation (0x02): the byte-order mark FF FE is stored FF 00 FE.
    let unsynced = unsynchronise(&body);
    assert_ne!(unsynced, body);
    // v2.4 grouping and data-length indicator (0x41): one group byte, four length bytes.
    let grouped = [&[7, 0, 0, 0, 9][..], &body].concat();
    // v2.3 grouping (0x20): one group byte.
    let grouped_v23 = [&[7][..], &body].concat();
    let comm_unsynced = unsynchronise(&comm_body(1, *b"eng", "iTunNORM", "x"));
    let v24 = tag(
        4,
        0,
        &[],
        &[
            frame(4, *b"GEOB", [0, 0x02], &unsynced),
            frame(4, *b"GEOB", [0, 0x41], &grouped),
            frame(4, *b"COMM", [0, 0x02], &comm_unsynced),
            frame(4, *b"GEOB", [0, 0x08], &body),
        ],
        0,
    );
    let index = parse_tag(&v24).expect("parses");
    for f in &index.frames[..2] {
        assert_eq!(f.description.as_deref(), Some("Serato Autotags"));
        assert_eq!(f.file_name.as_deref(), Some("f"));
    }
    assert_eq!(index.frames[2].description.as_deref(), Some("iTunNORM"));
    assert_eq!(index.frames[3].description, None, "compressed");
    let v23 = tag(
        3,
        0,
        &[],
        &[
            frame(3, *b"GEOB", [0, 0x20], &grouped_v23),
            frame(3, *b"GEOB", [0, 0x40], &body),
        ],
        0,
    );
    let index = parse_tag(&v23).expect("parses");
    assert_eq!(
        index.frames[0].description.as_deref(),
        Some("Serato Autotags")
    );
    assert_eq!(index.frames[1].description, None, "encrypted");
}

#[test]
fn txxx_values_are_read_from_the_tag() {
    let frames = [
        txxx(3, 0, "SOUNDCHECK", "v=1;app=0.1.0"),
        txxx(3, 1, "Wert", "Ünïcode"),
        txxx(3, 0, "EMPTY", ""),
        text(3, *b"TBPM", 0, "128"),
    ];
    let t = tag(3, 0, &[], &frames, 8);
    let index = parse_tag(&t).expect("parses");
    let value = |i: usize| index.txxx_value(&t, &index.frames[i]);
    assert_eq!(value(0).as_deref(), Some("v=1;app=0.1.0"));
    assert_eq!(value(1).as_deref(), Some("Ünïcode"));
    assert_eq!(value(2).as_deref(), Some(""));
    assert_eq!(value(3), None, "not a TXXX frame");
    let v24 = tag(4, 0, &[], &[txxx(4, 3, "SOUNDCHECK", "v=1;é")], 0);
    let index = parse_tag(&v24).expect("parses");
    assert_eq!(
        index.txxx_value(&v24, &index.frames[0]).as_deref(),
        Some("v=1;é")
    );
}
