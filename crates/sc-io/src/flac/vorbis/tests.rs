//! Unit tests of `vorbis.rs`: parsing, in-place replacement, appending, and the refusals.

use super::*;

fn comment(vendor: &str, fields: &[&[u8]]) -> Vec<u8> {
    let mut p = u32::try_from(vendor.len())
        .expect("short")
        .to_le_bytes()
        .to_vec();
    p.extend_from_slice(vendor.as_bytes());
    p.extend_from_slice(&u32::try_from(fields.len()).expect("few").to_le_bytes());
    for f in fields {
        p.extend_from_slice(&u32::try_from(f.len()).expect("short").to_le_bytes());
        p.extend_from_slice(f);
    }
    p
}

fn edit(name: &str, value: &str) -> VorbisEdit {
    VorbisEdit::new(name, value).expect("valid edit")
}

#[test]
fn replaces_in_place_under_the_existing_name_and_appends_the_rest() {
    let p = comment(
        "vendor \u{e9}",
        &[
            b"TITLE=A",
            b"replaygain_track_gain=-4.10 dB",
            b"NOEQUALS",
            b"ARTIST=B",
            b"REPLAYGAIN_TRACK_GAIN=-1 dB",
        ],
    );
    let edits = [
        edit("REPLAYGAIN_TRACK_GAIN", "-6.20 dB"),
        edit("SOUNDCHECK", "{}"),
    ];
    let out = edit_comment(&p, &edits).expect("edits");
    let want = comment(
        "vendor \u{e9}",
        &[
            b"TITLE=A",
            b"replaygain_track_gain=-6.20 dB",
            b"NOEQUALS",
            b"ARTIST=B",
            b"REPLAYGAIN_TRACK_GAIN=-6.20 dB",
            b"SOUNDCHECK={}",
        ],
    );
    assert_eq!(out.bytes, want);
    assert_eq!(
        out.summary,
        EditSummary {
            replaced: 2,
            appended: 1,
            comment_bytes: u32::try_from(want.len()).expect("short"),
        }
    );
}

#[test]
fn no_edits_reproduce_the_payload_and_fields_keep_invalid_utf8() {
    let p = comment("v", &[b"A=\xff\xfe", b"B=2"]);
    assert_eq!(edit_comment(&p, &[]).expect("parses").bytes, p);
    let idx = index(&p).expect("parses");
    assert_eq!(idx.fields.len(), 2);
    assert_eq!(CommentIndex::name(&p, &idx.fields[1]), Some(&b"B"[..]));
}

#[test]
fn malformed_payloads_are_not_edited() {
    let good = comment("v", &[b"A=1"]);
    let mut extra = good.clone();
    extra.push(0);
    let mut overrun = good.clone();
    let last = overrun.len() - 4;
    overrun[last - 4..last].copy_from_slice(&100_u32.to_le_bytes());
    let mut huge_count = comment("v", &[]);
    let n = huge_count.len();
    huge_count[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    for bad in [&extra[..], &overrun, &huge_count, &good[..3], &[]] {
        assert!(
            matches!(
                edit_comment(bad, &[edit("A", "2")]),
                Err(NotEditable::Malformed { .. })
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn edit_names_are_validated_and_unique() {
    for bad in ["", "A=B", "\u{e9}", "A~", "TAB\t"] {
        assert!(VorbisEdit::new(bad, "x").is_err(), "{bad:?}");
    }
    assert!(VorbisEdit::new("A", "nul\0").is_err());
    assert!(VorbisEdit::new("A", &"x".repeat(MAX_VALUE_BYTES + 1)).is_err());
    assert!(VorbisEdit::new(" !}", "ok").is_ok());
    let twice = [
        TagEdit {
            label: "soundcheck".into(),
            value: "1".into(),
        },
        TagEdit {
            label: "SOUNDCHECK".into(),
            value: "2".into(),
        },
    ];
    assert!(matches!(edits_from(&twice), Err(Error::InvalidArgument(_))));
    assert_eq!(edits_from(&twice[..1]).expect("valid").len(), 1);
}

#[test]
fn a_result_past_the_block_limit_is_too_large() {
    let big = "x".repeat(MAX_VALUE_BYTES);
    let fields: Vec<Vec<u8>> = (0..255)
        .map(|i| format!("F{i}={big}").into_bytes())
        .collect();
    let refs: Vec<&[u8]> = fields.iter().map(Vec::as_slice).collect();
    let p = comment("v", &refs);
    assert!(p.len() < MAX_BLOCK_BYTES as usize);
    let edits = [edit("NEW", &big)];
    assert!(matches!(
        edit_comment(&p, &edits),
        Err(NotEditable::TooLarge { .. })
    ));
}
