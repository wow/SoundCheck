//! Unit tests of `crates/sc-io/src/id3/edit.rs`: replacement in place, append order, padding
//! reuse and growth, extended headers, footers, the bytes after a tag, and a property test over
//! random frame lists and padding sizes.

use proptest::prelude::*;

use super::*;
use crate::id3::parse::{FrameRef, TagIndex};
use crate::id3::test_build::{ext_v23, ext_v24_update, frame, tag, tag_with_footer, text, txxx};

fn edits(list: &[(&str, &str)]) -> Vec<Edit> {
    list.iter()
        .map(|(l, v)| Edit::new(l, v).expect("valid edit"))
        .collect()
}

fn ours() -> Vec<Edit> {
    edits(&[
        ("TBPM", "128"),
        ("TXXX:BPM", "128.00"),
        ("TXXX:REPLAYGAIN_TRACK_GAIN", "-6.20 dB"),
        ("TXXX:REPLAYGAIN_TRACK_PEAK", "0.912345"),
        ("TXXX:SOUNDCHECK", &"{\"schema\":1}".repeat(20)),
    ])
}

fn frames_of<'a>(bytes: &'a [u8], index: &TagIndex) -> Vec<&'a [u8]> {
    index
        .frames
        .iter()
        .map(|f| &bytes[f.range.clone()])
        .collect()
}

fn find<'a>(index: &'a TagIndex, edit: &Edit) -> Vec<&'a FrameRef> {
    index.frames.iter().filter(|f| edit.matches(f)).collect()
}

#[test]
fn existing_frames_are_replaced_in_place_and_neighbours_kept_byte_for_byte() {
    for major in [3, 4] {
        let neighbours = [
            text(major, *b"TIT2", 0, "Matrix Tone"),
            text(major, *b"TBPM", 0, "127"),
            txxx(major, 1, "replaygain_track_gain", "-4.10 dB"),
            frame(major, *b"PRIV", [0x40, 0], b"owner\0\xFF\x00\x01"),
        ];
        let t = tag(major, 0, &[], &neighbours, 512);
        let out = edit_tag(&t, &ours()).expect("editable");
        let index = parse_tag(&out.bytes).expect("parses");
        let got = frames_of(&out.bytes, &index);
        assert_eq!(got.len(), 7);
        assert_eq!(got[0], &neighbours[0][..]);
        assert_eq!(got[1], &ours()[0].frame(major)[..], "TBPM in place");
        assert_eq!(got[2], &ours()[2].frame(major)[..], "ReplayGain in place");
        assert_eq!(got[3], &neighbours[3][..]);
        // The rest appended in request order.
        for (k, e) in [1, 3, 4].iter().enumerate() {
            assert_eq!(got[4 + k], &ours()[*e].frame(major)[..]);
        }
        assert_eq!((out.summary.replaced, out.summary.appended), (2, 3));
        assert_eq!(out.bytes.len(), t.len(), "the padding held the new frames");
        assert!(!out.summary.grew);
        assert_eq!(&out.bytes[..6], &t[..6], "version, revision, flags");
    }
}

#[test]
fn txxx_descriptions_match_case_insensitively_in_every_encoding() {
    for major in [3, 4] {
        for enc in 0..=3 {
            let t = tag(
                major,
                0,
                &[],
                &[txxx(major, enc, "ReplayGain_Track_Peak", "1.0")],
                64,
            );
            let out = edit_tag(&t, &ours()).expect("editable");
            let index = parse_tag(&out.bytes).expect("parses");
            let peak = &ours()[3];
            assert_eq!(find(&index, peak).len(), 1, "v2.{major} encoding {enc}");
            assert_eq!(index.frames[0].range.start, 10, "replaced in place");
            assert_eq!(out.summary.replaced, 1);
        }
    }
}

#[test]
fn every_frame_with_an_edited_label_is_replaced() {
    let t = tag(
        3,
        0,
        &[],
        &[
            txxx(3, 0, "replaygain_track_gain", "-1 dB"),
            text(3, *b"TIT2", 0, "x"),
            txxx(3, 0, "REPLAYGAIN_TRACK_GAIN", "-2 dB"),
        ],
        0,
    );
    let gain = edits(&[("TXXX:REPLAYGAIN_TRACK_GAIN", "-6.20 dB")]);
    let out = edit_tag(&t, &gain).expect("editable");
    let index = parse_tag(&out.bytes).expect("parses");
    let got = frames_of(&out.bytes, &index);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], &gain[0].frame(3)[..]);
    assert_eq!(got[2], &gain[0].frame(3)[..]);
    assert_eq!((out.summary.replaced, out.summary.appended), (2, 0));
}

#[test]
fn padding_is_reused_when_it_fits_and_zero_filled() {
    let frames = [text(4, *b"TBPM", 0, "127.5"), text(4, *b"TIT2", 0, "A")];
    let t = tag(4, 0, &[], &frames, 40);
    // A shorter replacement leaves more padding; the tag keeps its size.
    let out = edit_tag(&t, &edits(&[("TBPM", "128")])).expect("editable");
    assert_eq!(out.bytes.len(), t.len());
    let index = parse_tag(&out.bytes).expect("parses");
    assert_eq!(index.padding.len(), 42);
    assert!(out.bytes[index.padding.clone()].iter().all(|b| *b == 0));
    assert_eq!(out.summary.padding_bytes, 42);
    // Exactly filling the padding still fits.
    // TXXX header 10, encoding 1, "BPM" 3, terminator 1: a 25-byte value fills 40 bytes.
    let exact = "x".repeat(40 - 10 - 5);
    let out = edit_tag(&t, &edits(&[("TXXX:BPM", &exact)])).expect("editable");
    assert_eq!(out.bytes.len(), t.len());
    assert_eq!(out.summary.padding_bytes, 0);
    assert!(!out.summary.grew);
}

#[test]
fn a_tag_without_room_grows_with_fresh_padding() {
    let frames = [text(3, *b"TIT2", 0, "A")];
    let t = tag(3, 0, &[], &frames, 4);
    let out = edit_tag(&t, &ours()).expect("editable");
    assert!(out.summary.grew);
    let index = parse_tag(&out.bytes).expect("parses");
    assert_eq!(index.padding.len(), GROWTH_PADDING_BYTES);
    assert_eq!(out.summary.tag_bytes as usize, out.bytes.len());
    assert!(out.bytes[index.padding.clone()].iter().all(|b| *b == 0));
    // A second edit with values of the same length now fits in place.
    let again = edit_tag(&out.bytes, &ours()).expect("editable");
    assert_eq!(again.bytes, out.bytes);
    assert!(!again.summary.grew);
}

#[test]
fn a_v23_extended_header_gets_the_new_padding_size_and_nothing_else() {
    let frames = [text(3, *b"TIT2", 0, "A")];
    let ext = ext_v23(1000);
    let t = tag(3, 0x40, &ext, &frames, 1000);
    let out = edit_tag(&t, &ours()).expect("editable");
    let index = parse_tag(&out.bytes).expect("parses");
    let new_ext = &out.bytes[index.extended.clone().expect("kept")];
    assert_eq!(&new_ext[..6], &ext[..6]);
    let padding = u32::try_from(index.padding.len()).expect("small");
    assert_eq!(new_ext[6..10], padding.to_be_bytes());
    assert_eq!(out.bytes.len(), t.len());
}

#[test]
fn a_v24_extended_header_is_carried_verbatim() {
    let ext = ext_v24_update();
    let t = tag(4, 0x40, &ext, &[text(4, *b"TIT2", 3, "A")], 0);
    let out = edit_tag(&t, &ours()).expect("editable");
    assert_eq!(&out.bytes[10..16], &ext[..]);
    assert_eq!(out.bytes[5], 0x40);
    assert!(out.summary.grew);
}

#[test]
fn a_footer_is_rewritten_with_the_new_size_and_no_padding() {
    let t = tag_with_footer(&[text(4, *b"TIT2", 3, "A")]);
    let out = edit_tag(&t, &ours()).expect("editable");
    let index = parse_tag(&out.bytes).expect("parses");
    assert!(index.footer && index.padding.is_empty());
    let n = out.bytes.len();
    assert_eq!(&out.bytes[n - 10..n - 7], b"3DI");
    assert_eq!(out.bytes[n - 7..n], out.bytes[3..10]);
    assert_eq!(out.summary.padding_bytes, 0);
}

#[test]
fn bytes_after_the_tag_in_its_chunk_are_carried_after_it() {
    let mut chunk = tag(3, 0, &[], &[text(3, *b"TIT2", 0, "A")], 0);
    let tail = [0xAB, 0, 0xCD];
    chunk.extend_from_slice(&tail);
    let out = edit_tag(&chunk, &ours()).expect("editable");
    assert!(out.bytes.ends_with(&tail));
    assert_eq!(out.summary.tag_bytes as usize + tail.len(), out.bytes.len());
}

#[test]
fn an_empty_tag_gets_every_edit_appended() {
    let t = tag(3, 0, &[], &[], 0);
    let out = edit_tag(&t, &ours()).expect("editable");
    let index = parse_tag(&out.bytes).expect("parses");
    assert_eq!(index.frames.len(), 5);
    assert_eq!(out.summary.appended, 5);
}

#[test]
fn an_unreadable_txxx_description_stops_txxx_edits_only() {
    for (major, flags) in [
        (3, [0, 0x80]),
        (3, [0, 0x40]),
        (4, [0, 0x08]),
        (4, [0, 0x04]),
    ] {
        let hidden = frame(major, *b"TXXX", flags, b"\0\0\0\x10x\x9c\x01\x02");
        let t = tag(major, 0, &[], &[text(major, *b"TIT2", 0, "A"), hidden], 64);
        assert_eq!(edit_tag(&t, &ours()), Err(NotEditable::UnreadableTxxx));
        let out = edit_tag(&t, &edits(&[("TBPM", "128")])).expect("text frame edits go on");
        assert_eq!(out.summary.appended, 1);
    }
    let unknown = frame(3, *b"TXXX", [0, 0], b"\x07BPM\x001");
    let t = tag(3, 0, &[], &[unknown], 64);
    assert_eq!(edit_tag(&t, &ours()), Err(NotEditable::UnreadableTxxx));
}

#[test]
fn a_bom_less_utf16_description_in_big_endian_is_replaced_not_duplicated() {
    let mut body = vec![1];
    body.extend(crate::id3::test_build::encode(
        2,
        "replaygain_track_gain",
        true,
    ));
    body.extend(crate::id3::test_build::encode(2, "-1 dB", false));
    let t = tag(3, 0, &[], &[frame(3, *b"TXXX", [0, 0], &body)], 512);
    let out = edit_tag(&t, &ours()).expect("editable");
    assert_eq!((out.summary.replaced, out.summary.appended), (1, 4));
}

#[test]
fn tags_that_are_not_editable_are_reported_not_rewritten() {
    let unsync = tag(4, 0x80, &[], &[text(4, *b"TIT2", 0, "A")], 0);
    assert_eq!(edit_tag(&unsync, &ours()), Err(NotEditable::Unsynchronised));
    let near_cap = usize::try_from(MAX_TAG_BYTES).expect("fits") - 20;
    let big = tag(
        4,
        0,
        &[],
        &[frame(4, *b"PRIV", [0, 0], &vec![1; near_cap])],
        0,
    );
    assert!(matches!(
        edit_tag(&big, &ours()),
        Err(NotEditable::TooLarge { .. })
    ));
}

/// Format flags that keep a frame's text readable: v2.3 grouping (0x20); v2.4 grouping (0x40),
/// unsynchronisation (0x02), data-length indicator (0x01) and grouping with the indicator.
fn arb_format(major: u8) -> impl Strategy<Value = u8> {
    if major == 3 {
        prop::sample::select(vec![0, 0x20])
    } else {
        prop::sample::select(vec![0, 0x40, 0x01, 0x02, 0x41])
    }
}

/// Frame data as stored under `format`: the group byte and the v2.4 data-length indicator in
/// front, and v2.4 unsynchronisation over all of it.
fn stored(major: u8, format: u8, group: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if format & (if major == 3 { 0x20 } else { 0x40 }) != 0 {
        out.push(group);
    }
    if major == 4 && format & 0x01 != 0 {
        let len = u32::try_from(data.len()).expect("small");
        out.extend(crate::id3::encode_syncsafe(len).expect("small"));
    }
    out.extend_from_slice(data);
    if major == 4 && format & 0x02 != 0 {
        let mut unsynced = Vec::with_capacity(out.len() + 8);
        for b in out {
            unsynced.push(b);
            if b == 0xFF {
                unsynced.push(0);
            }
        }
        out = unsynced;
    }
    out
}

/// Text for `encoding`: Latin-1 for 0, any of ASCII, Latin-1, Turkish, the euro sign and an
/// emoji (a UTF-16 surrogate pair) otherwise.
fn arb_text(encoding: u8, max: usize) -> BoxedStrategy<String> {
    let chars = if encoding == 0 {
        "[ -~à-ÿ]"
    } else {
        "[ -~é€ışĞ😀]"
    };
    proptest::string::string_regex(&format!("{chars}{{0,{max}}}"))
        .expect("valid regex")
        .boxed()
}

/// A random frame of a tag of `major` with random status flags and readable format flags
/// (group byte, data-length indicator, unsynchronisation): an opaque frame with a common id,
/// or a `TXXX` frame in any encoding whose description may be one of ours in another letter
/// case and whose value may hold non-ASCII text.
fn arb_frame(major: u8) -> impl Strategy<Value = Vec<u8>> {
    let ids = prop::sample::select(vec![*b"TIT2", *b"TBPM", *b"PRIV", *b"GEOB", *b"APIC"]);
    let descs = prop::sample::select(vec![
        "SOURCE",
        "replaygain_track_gain",
        "Soundcheck",
        "bpm",
        "Serato Markers2",
        "ÄRGER",
    ]);
    let txxx_frame = (0_u8..=3, descs).prop_flat_map(move |(enc, desc)| {
        (
            Just(enc),
            Just(desc),
            arb_text(enc, 40),
            any::<u8>(),
            arb_format(major),
            any::<u8>(),
        )
    });
    prop_oneof![
        (
            ids,
            any::<u8>(),
            arb_format(major),
            any::<u8>(),
            prop::collection::vec(any::<u8>(), 1..300)
        )
            .prop_map(move |(id, status, format, group, body)| {
                frame(
                    major,
                    id,
                    [status, format],
                    &stored(major, format, group, &body),
                )
            }),
        txxx_frame.prop_map(move |(enc, desc, value, status, format, group)| {
            let mut data = vec![enc];
            data.extend(crate::id3::test_build::encode(enc, desc, true));
            data.extend(crate::id3::test_build::encode(enc, &value, false));
            frame(
                major,
                *b"TXXX",
                [status, format],
                &stored(major, format, group, &data),
            )
        }),
    ]
}

fn ambiguous(r: &Result<TagIndex, NotEditable>) -> bool {
    matches!(r, Err(NotEditable::Malformed { detail }) if detail.contains("ambiguous"))
}

proptest! {
    #[test]
    fn edits_keep_every_other_frame_in_order_and_reparse(
        (major, frames) in (3_u8..=4)
            .prop_flat_map(|m| (Just(m), prop::collection::vec(arb_frame(m), 0..12))),
        padding in 0_usize..600,
        pick in prop::collection::vec(any::<bool>(), 5),
        record in arb_text(1, 300),
    ) {
        let chosen: Vec<Edit> = edits(&[
            ("TBPM", "128"),
            ("TXXX:BPM", "128.00"),
            ("TXXX:REPLAYGAIN_TRACK_GAIN", "-6.20 dB"),
            ("TXXX:REPLAYGAIN_TRACK_PEAK", "0.912345"),
            ("TXXX:SOUNDCHECK", &record),
        ])
        .into_iter()
        .zip(&pick)
        .filter_map(|(e, keep)| keep.then_some(e))
        .collect();
        let t = tag(major, 0, &[], &frames, padding);
        let before = parse_tag(&t);
        if ambiguous(&before) {
            // A v2.4 binary last frame of 128+ bytes before enough padding: refused as
            // ambiguous (see parse.rs), never edited.
            prop_assert!(edit_tag(&t, &chosen).is_err());
            return Ok(());
        }
        let before = before.expect("generated tags parse");
        let out = edit_tag(&t, &chosen).expect("editable");
        let after = parse_tag(&out.bytes);
        let appended = chosen
            .iter()
            .filter(|e| !before.frames.iter().any(|f| e.matches(f)))
            .count();
        if ambiguous(&after) {
            // Only when nothing was appended and a shorter replacement freed padding behind
            // such a frame.
            prop_assert_eq!(appended, 0);
            return Ok(());
        }
        let after = after.expect("edited tags parse");
        prop_assert_eq!((after.major, after.revision, after.flags), (major, 0, 0));
        let old = frames_of(&t, &before);
        let new = frames_of(&out.bytes, &after);
        prop_assert_eq!(new.len(), old.len() + appended);
        for (k, f) in before.frames.iter().enumerate() {
            if let Some(e) = chosen.iter().find(|e| e.matches(f)) {
                prop_assert_eq!(new[k], &e.frame(major)[..]);
            } else {
                prop_assert_eq!(new[k], old[k]);
            }
        }
        let tail: Vec<&Edit> = chosen
            .iter()
            .filter(|e| !before.frames.iter().any(|f| e.matches(f)))
            .collect();
        for (k, e) in tail.iter().enumerate() {
            prop_assert_eq!(new[old.len() + k], &e.frame(major)[..]);
            prop_assert!(e.matches(&after.frames[old.len() + k]), "ours re-read");
        }
        let grown = new.iter().map(|f| f.len()).sum::<usize>()
            .saturating_sub(old.iter().map(|f| f.len()).sum::<usize>());
        prop_assert_eq!(out.summary.grew, grown > padding);
        if grown <= padding {
            prop_assert_eq!(out.bytes.len(), t.len());
        }
        prop_assert!(out.bytes[after.padding.clone()].iter().all(|b| *b == 0));
        // Editing again with the same edits changes nothing.
        let again = edit_tag(&out.bytes, &chosen).expect("editable");
        prop_assert_eq!(again.bytes, out.bytes);
    }
}
