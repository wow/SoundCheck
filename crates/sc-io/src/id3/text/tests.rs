//! Unit tests of `crates/sc-io/src/id3/text.rs`: text decoding in the four encodings, `TXXX`
//! descriptions behind frame format flags, edit validation and the frames SoundCheck writes.

use super::*;
use crate::id3::parse::decode_syncsafe;
use crate::id3::test_build::encode;

/// The little-endian (or only) reading of a `TXXX` description.
fn txxx_description(major: u8, flags: [u8; 2], body: &[u8]) -> Option<String> {
    txxx_descriptions(major, flags, body).map(|(d, _)| d)
}

#[test]
fn text_decodes_in_all_four_encodings_up_to_the_terminator() {
    for enc in 0..=3 {
        let mut b = encode(enc, "ReplayGain_Track_Gain", true);
        b.extend(encode(enc, "-4.10 dB", false));
        assert_eq!(
            decode_text(enc, &b).as_deref(),
            Some("ReplayGain_Track_Gain"),
            "encoding {enc}"
        );
    }
    // Non-ASCII text in the encodings that hold it.
    for enc in 1..=3 {
        let b = encode(enc, "Sarı Çizmeli", true);
        assert_eq!(decode_text(enc, &b).as_deref(), Some("Sarı Çizmeli"));
    }
    assert_eq!(decode_text(0, &[0xC7, 0x41, 0]).as_deref(), Some("ÇA"));
    assert_eq!(decode_text(4, b"x\0"), None);
}

#[test]
fn utf16_follows_its_byte_order_mark_and_defaults_to_little_endian() {
    let big = [0xFE, 0xFF, 0, b'B', 0, b'P', 0, b'M', 0, 0];
    assert_eq!(decode_text(1, &big).as_deref(), Some("BPM"));
    let little = [0xFF, 0xFE, b'B', 0, b'P', 0, 0, 0];
    assert_eq!(decode_text(1, &little).as_deref(), Some("BP"));
    let bare = [b'B', 0, b'P', 0];
    assert_eq!(decode_text(1, &bare).as_deref(), Some("BP"));
    // An unterminated description ends with the body; an odd last byte is ignored.
    assert_eq!(decode_text(2, &[0, b'X', 0]).as_deref(), Some("X"));
}

#[test]
fn txxx_descriptions_skip_format_flag_data_and_undo_v24_unsynchronisation() {
    let mut body = vec![0];
    body.extend(b"SOUNDCHECK\0{}");
    assert_eq!(
        txxx_description(3, [0, 0], &body).as_deref(),
        Some("SOUNDCHECK")
    );
    // v2.3 grouping: one group byte first.
    let grouped = [&[7][..], &body].concat();
    assert_eq!(
        txxx_description(3, [0, 0x20], &grouped).as_deref(),
        Some("SOUNDCHECK")
    );
    // v2.3 compression or encryption: unreadable.
    assert_eq!(txxx_description(3, [0, 0x80], &body), None);
    assert_eq!(txxx_description(3, [0, 0x40], &body), None);
    // v2.4 grouping byte, then the data-length indicator.
    let dli = [&[7, 0, 0, 0, 13][..], &body].concat();
    assert_eq!(
        txxx_description(4, [0, 0x41], &dli).as_deref(),
        Some("SOUNDCHECK")
    );
    // v2.4 unsynchronisation: a 0x00 after each 0xFF is dropped (UTF-16 with BOM FF FE).
    let mut utf16 = vec![1];
    utf16.extend(encode(1, "BPM", true));
    let mut unsynced = Vec::new();
    for b in &utf16 {
        unsynced.push(*b);
        if *b == 0xFF {
            unsynced.push(0);
        }
    }
    assert_ne!(unsynced, utf16);
    assert_eq!(
        txxx_description(4, [0, 0x02], &unsynced).as_deref(),
        Some("BPM")
    );
    assert_eq!(txxx_description(4, [0, 0x08], &body), None);
    assert_eq!(txxx_description(4, [0, 0x04], &body), None);
    assert_eq!(txxx_description(4, [0, 0x01], &[0, 0]), None);
}

#[test]
fn edits_accept_text_frames_and_txxx_descriptions_only() {
    assert_eq!(
        Edit::new("TBPM", "128").expect("valid").label(),
        &Label::Frame(*b"TBPM")
    );
    assert_eq!(
        Edit::new("TXXX:replaygain_track_gain", "-1 dB")
            .expect("valid")
            .label(),
        &Label::Txxx("replaygain_track_gain".into())
    );
    for bad in [
        "TXXX",
        "TXXX:",
        "APIC",
        "TBP",
        "Tbpm",
        "COMM:x",
        "TXXX:a\0b",
    ] {
        assert!(Edit::new(bad, "1").is_err(), "{bad:?}");
    }
    assert!(Edit::new("TBPM", "1\u{0}2").is_err());
    assert!(Edit::new("TBPM", &"9".repeat(MAX_VALUE_BYTES + 1)).is_err());
    assert!(Edit::new(&format!("TXXX:{}", "D".repeat(257)), "1").is_err());
}

#[test]
fn edits_with_the_same_label_are_refused() {
    let edit = |label: &str| TagEdit {
        label: label.into(),
        value: "1".into(),
    };
    let ok = edits_from(&[edit("TBPM"), edit("TXXX:BPM"), edit("TXXX:SOUNDCHECK")]);
    assert_eq!(ok.expect("valid").len(), 3);
    assert!(edits_from(&[edit("TXXX:BPM"), edit("TXXX:bpm")]).is_err());
    assert!(edits_from(&[edit("TBPM"), edit("TBPM")]).is_err());
}

fn frame_size(major: u8, frame: &[u8]) -> usize {
    let b = [frame[4], frame[5], frame[6], frame[7]];
    let v = if major == 3 {
        u32::from_be_bytes(b)
    } else {
        decode_syncsafe(b).expect("syncsafe")
    };
    usize::try_from(v).expect("small")
}

#[test]
fn our_frames_use_latin1_for_ascii_else_utf16_in_v23_and_utf8_in_v24() {
    let ascii = Edit::new("TXXX:BPM", "128.00").expect("valid");
    for major in [3, 4] {
        let f = ascii.frame(major);
        assert_eq!(&f[..4], b"TXXX");
        assert_eq!(&f[8..10], [0, 0], "flags 0");
        assert_eq!(&f[10..], b"\x00BPM\x00\x31\x32\x38.00");
        assert_eq!(frame_size(major, &f), f.len() - 10);
    }
    let text = Edit::new("TXXX:NOTE", "Sarı").expect("valid");
    let v23 = text.frame(3);
    assert_eq!(v23[10], 1);
    let mut want = vec![1];
    want.extend(encode(1, "NOTE", true));
    want.extend(encode(1, "Sarı", false));
    assert_eq!(&v23[10..], &want[..]);
    let v24 = text.frame(4);
    assert_eq!(&v24[10..], "\u{3}NOTE\0Sarı".as_bytes());

    let tbpm = Edit::new("TBPM", "128").expect("valid").frame(4);
    assert_eq!(tbpm, [&b"TBPM\0\0\0\x04\0\0\0"[..], b"128"].concat());
}

#[test]
fn a_frame_over_127_bytes_has_different_v23_and_v24_size_fields() {
    let long = Edit::new("TXXX:SOUNDCHECK", &"x".repeat(250)).expect("valid");
    let (v23, v24) = (long.frame(3), long.frame(4));
    assert_eq!(v23[10..], v24[10..]);
    assert_ne!(v23[4..8], v24[4..8]);
    assert_eq!(frame_size(3, &v23), frame_size(4, &v24));
}

#[test]
fn txxx_matches_ignore_ascii_case_and_text_frames_match_by_id() {
    let frame = |id: &[u8; 4], desc: Option<&str>| FrameRef {
        id: *id,
        flags: [0, 0],
        range: 0..10,
        description: desc.map(str::to_string),
        description_be: None,
    };
    let rg = Edit::new("TXXX:REPLAYGAIN_TRACK_GAIN", "x").expect("valid");
    assert!(rg.matches(&frame(b"TXXX", Some("replaygain_track_gain"))));
    assert!(rg.matches(&frame(b"TXXX", Some("ReplayGain_Track_Gain"))));
    assert!(!rg.matches(&frame(b"TXXX", Some("REPLAYGAIN_TRACK_PEAK"))));
    assert!(!rg.matches(&frame(b"TXXX", None)));
    assert!(!rg.matches(&frame(b"COMM", Some("REPLAYGAIN_TRACK_GAIN"))));
    let bpm = Edit::new("TBPM", "x").expect("valid");
    assert!(bpm.matches(&frame(b"TBPM", None)));
    assert!(!bpm.matches(&frame(b"TKEY", None)));
}

#[test]
fn utf16_without_a_byte_order_mark_is_read_in_both_orders() {
    let le: Vec<u8> = [&[1][..], b"B\0P\0M\0\0\0"].concat();
    let be: Vec<u8> = [&[1][..], b"\0B\0P\0M\0\0"].concat();
    assert_eq!(
        txxx_descriptions(3, [0, 0], &le),
        Some(("BPM".into(), Some("\u{4200}\u{5000}\u{4d00}".into())))
    );
    let (little, big) = txxx_descriptions(4, [0, 0], &be).expect("readable");
    assert_eq!(big.as_deref(), Some("BPM"));
    assert_ne!(little, "BPM");
    let bpm = Edit::new("TXXX:bpm", "1").expect("valid");
    let frame = FrameRef {
        id: *b"TXXX",
        flags: [0, 0],
        range: 0..10,
        description: Some(little),
        description_be: big,
    };
    assert!(bpm.matches(&frame));
    // With a byte-order mark the order is known: no second reading.
    let marked: Vec<u8> = [&[1][..], &encode(1, "BPM", true)].concat();
    assert_eq!(
        txxx_descriptions(3, [0, 0], &marked).and_then(|d| d.1),
        None
    );
}

#[test]
fn an_empty_txxx_body_is_an_empty_description_and_unknown_encodings_are_unreadable() {
    assert_eq!(
        txxx_descriptions(3, [0, 0], &[]),
        Some((String::new(), None))
    );
    assert_eq!(txxx_descriptions(3, [0, 0], b"\x04BPM\0x"), None);
}
