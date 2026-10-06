//! Unit tests of `crates/sc-io/src/id3/mod.rs`: the reasons a tag is not edited read as plain
//! sentences, and the public entry points work together on a typical DJ tag.

use super::test_build::{frame, tag, text, txxx};
use super::*;

#[test]
fn reasons_read_as_sentences() {
    let cases = [
        (NotEditable::NoTag, "the file has no ID3 tag"),
        (
            NotEditable::SeveralTags { count: 2 },
            "the file has 2 ID3 tags",
        ),
        (
            NotEditable::UnsupportedVersion { major: 2 },
            "ID3v2.2 tags are not edited",
        ),
        (NotEditable::Unsynchronised, "the tag is unsynchronised"),
        (
            NotEditable::UnknownFlags { flags: 0x08 },
            "the tag header has undefined flags (0x08)",
        ),
        (
            NotEditable::ExtendedHeaderCrc,
            "the tag is protected by a CRC",
        ),
        (NotEditable::Restricted, "the tag declares restrictions"),
        (
            NotEditable::TooLarge { bytes: 9 },
            "the tag is too large to edit (9 bytes)",
        ),
        (NotEditable::malformed("x"), "the tag is malformed: x"),
    ];
    for (reason, text) in cases {
        assert_eq!(reason.to_string(), text);
    }
}

#[test]
fn a_serato_style_tag_gets_our_frames_and_keeps_its_objects() {
    let geob = |desc: &str| {
        let mut body = b"\0application/octet-stream\0\0".to_vec();
        body.extend_from_slice(desc.as_bytes());
        body.extend_from_slice(&[0, 1, 1, 0xFF, 0x00, 0xE0]);
        frame(3, *b"GEOB", [0, 0], &body)
    };
    let frames = [
        text(3, *b"TIT2", 0, "Matrix Tone"),
        geob("Serato Markers2"),
        txxx(3, 1, "SOURCE", "synthetic"),
        geob("Serato BeatGrid"),
    ];
    let t = tag(3, 0, &[], &frames, 256);
    let request = [
        sc_core::TagEdit {
            label: "TBPM".into(),
            value: "128".into(),
        },
        sc_core::TagEdit {
            label: "TXXX:SOUNDCHECK".into(),
            value: "{\"schema\":1}".into(),
        },
    ];
    let edits = edits_from(&request).expect("valid edits");
    let out = edit_tag(&t, &edits).expect("editable");
    let index = parse_tag(&out.bytes).expect("parses");
    for (k, f) in frames.iter().enumerate() {
        assert_eq!(&out.bytes[index.frames[k].range.clone()], &f[..]);
    }
    assert_eq!(&index.frames[4].id, b"TBPM");
    assert_eq!(index.frames[5].description.as_deref(), Some("SOUNDCHECK"));
    assert_eq!(out.bytes.len(), t.len());
}
