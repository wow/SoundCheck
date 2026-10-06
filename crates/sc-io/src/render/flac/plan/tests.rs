//! Unit tests of `plan.rs`: fates, PADDING absorbing the comment's growth, the tag outcomes,
//! and the refusals that come from the blocks.

use std::io::Cursor;
use std::path::Path;

use sc_core::Error;

use super::*;
use crate::flac::read_layout;
use crate::flac::test_build::{encode, file, samples, vorbis};
use crate::flac::vorbis::VorbisEdit;

fn plan_of(blocks: &[(u8, Vec<u8>)], trim: u64, edits: &[VorbisEdit]) -> Result<Plan> {
    let data = samples(5000, 1, 16, 21);
    let (frames, info) = encode(&data, 44_100, 1, 16);
    let bytes = file(&[], &info, blocks, &frames, &[]);
    let path = Path::new("p.flac");
    let mut src = Cursor::new(bytes);
    let layout = read_layout(&mut src, path)?;
    plan(
        &mut src,
        path,
        &layout,
        PlanArgs {
            trim_frames: trim,
            frames_out: 5000 - trim,
            edits,
            gain_changed: false,
        },
    )
}

fn fates(p: &Plan) -> Vec<(u8, BlockFate)> {
    p.records
        .iter()
        .map(|r| match r.id {
            BlockId::FlacBlock(t) => (t, r.fate),
            BlockId::Chunk(_) => panic!("a FLAC plan lists blocks"),
        })
        .collect()
}

fn edit() -> Vec<VorbisEdit> {
    vec![VorbisEdit::new("SOUNDCHECK", "0123456789").expect("valid")]
}

/// A seek table with one real point and one placeholder.
fn table() -> Vec<u8> {
    let mut t = vec![0; 18];
    t[16] = 0x10;
    t.extend_from_slice(&u64::MAX.to_be_bytes());
    t.extend_from_slice(&[0; 10]);
    t
}

#[test]
fn every_block_gets_its_fate_and_padding_absorbs_the_growth() {
    use BlockFate::{Carried, Edited, Replaced};
    let comment = vorbis("v", &["TITLE=x"]);
    let blocks = [
        (3, table()),
        (4, comment.clone()),
        (2, b"riffdata".to_vec()),
        (6, vec![5; 30]),
        (99, vec![1, 2]),
        (1, vec![0; 100]),
        (1, vec![7; 10]),
    ];
    let p = plan_of(&blocks, 0, &edit()).expect("plans");
    assert_eq!(
        fates(&p),
        [
            (0, Replaced),
            (3, Replaced),
            (4, Edited),
            (2, Carried),
            (6, Carried),
            (99, Carried),
            (1, Replaced),
            (1, Replaced)
        ]
    );
    // "SOUNDCHECK=0123456789" is 21 bytes plus its 4-byte length.
    let grown = u32::try_from(comment.len()).expect("small") + 25;
    let lens: Vec<u32> = p.blocks.iter().map(|b| b.len).collect();
    assert_eq!(lens, [34, 36, grown, 8, 30, 2, 75, 10]);
    assert_eq!(p.blocks[6].body, Body::Zeros(75));
    assert_eq!(
        p.blocks[7].body,
        Body::Zeros(10),
        "a second PADDING is zero-filled"
    );
    assert!(matches!(p.tags, TagOutcome::Edited(s) if s.appended == 1 && s.replaced == 0));
    assert_eq!(
        p.metadata_bytes(),
        4 + lens.iter().map(|l| 4 + u64::from(*l)).sum::<u64>()
    );
}

#[test]
fn padding_clamps_at_zero_and_grows_when_the_comment_shrinks() {
    let small_pad = [(4, vorbis("v", &[])), (1, vec![0; 5])];
    let p = plan_of(&small_pad, 0, &edit()).expect("plans");
    assert_eq!(p.blocks[2].body, Body::Zeros(0));
    let long = format!("SOUNDCHECK={}", "y".repeat(50));
    let shrink = [(4, vorbis("v", &[&long])), (1, vec![0; 5])];
    let p = plan_of(&shrink, 0, &edit()).expect("plans");
    assert_eq!(
        p.blocks[2].body,
        Body::Zeros(5 + 40),
        "the comment lost 40 bytes"
    );
    assert!(matches!(p.tags, TagOutcome::Edited(s) if s.replaced == 1));
}

#[test]
fn tag_outcomes_without_one_editable_comment() {
    let none = plan_of(&[(1, vec![0; 4])], 0, &edit()).expect("plans");
    assert_eq!(
        none.tags,
        TagOutcome::NotAdded(NotEditable::NoVorbisComment)
    );
    let two = [(4, vorbis("a", &[])), (4, vorbis("b", &[]))];
    let p = plan_of(&two, 0, &edit()).expect("plans");
    assert_eq!(
        p.tags,
        TagOutcome::NotAdded(NotEditable::SeveralVorbisComments { count: 2 })
    );
    assert!(
        p.records
            .iter()
            .skip(1)
            .all(|r| r.fate == BlockFate::Carried)
    );
    let mut bad = vorbis("v", &["A=1"]);
    bad.push(0);
    let p = plan_of(&[(4, bad)], 0, &edit()).expect("plans");
    assert!(matches!(
        p.tags,
        TagOutcome::NotAdded(NotEditable::Malformed { .. })
    ));
    assert_eq!(p.records[1].fate, BlockFate::Carried);
    let p = plan_of(&[(4, vorbis("v", &[]))], 0, &[]).expect("plans");
    assert_eq!(p.tags, TagOutcome::NotRequested);
    assert_eq!(p.records[1].fate, BlockFate::Carried);
}

/// A CUESHEET with one track at 0 (index 01 at 0) and the lead-out at 5000.
fn cuesheet(cdda: bool) -> Vec<u8> {
    let mut p = vec![0_u8; 396];
    p[136] = if cdda { 0x80 } else { 0 };
    p[395] = 2;
    let mut track = |offset: u64, number: u8, index: bool| {
        p.extend_from_slice(&offset.to_be_bytes());
        p.push(number);
        p.extend_from_slice(&[0; 26]);
        p.push(u8::from(index));
        if index {
            p.extend_from_slice(&[0; 8]);
            p.extend_from_slice(&[1, 0, 0, 0]);
        }
    };
    track(0, 1, true);
    track(5000, if cdda { 170 } else { 255 }, false);
    p
}

#[test]
fn cue_sheets_are_patched_under_a_trim_and_refused_when_they_cannot_be() {
    let p = plan_of(&[(5, cuesheet(false))], 441, &[]).expect("plans");
    assert_eq!(p.records[1].fate, BlockFate::Patched);
    let Body::Bytes(b) = &p.blocks[1].body else {
        panic!("patched bytes")
    };
    assert_eq!(
        &b[396 + 48..396 + 56],
        &4559_u64.to_be_bytes(),
        "the lead-out"
    );
    let carried = plan_of(&[(5, cuesheet(false))], 0, &[]).expect("plans");
    assert_eq!(carried.records[1].fate, BlockFate::Carried);
    let cdda = plan_of(&[(5, cuesheet(true))], 441, &[]);
    assert!(matches!(cdda, Err(Error::InvalidArgument(_))));
    let short = plan_of(&[(5, vec![0; 100])], 441, &[]);
    assert!(matches!(short, Err(Error::Corrupt { .. })));
}

#[test]
fn a_partial_seek_point_is_left_out_and_padding_takes_its_bytes() {
    let mut odd = table();
    odd.extend_from_slice(&[0xAB; 7]);
    let p = plan_of(&[(3, odd), (1, vec![0; 10])], 0, &[]).expect("plans");
    assert_eq!(
        p.blocks[1].body,
        Body::SeekTable(SeekShape {
            real: 1,
            placeholders: 1
        })
    );
    assert_eq!((p.blocks[1].len, p.blocks[2].len), (36, 17));
    assert_eq!(p.records[1].fate, BlockFate::Replaced);
}
