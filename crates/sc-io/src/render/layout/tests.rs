//! Unit tests of `crates/sc-io/src/render/layout.rs`: size limits, the patched-bytes budget,
//! sampler loops and `bext` versions, on files built in memory.
use std::io::Cursor;
use std::path::Path;

use sc_core::Error;

use super::super::tests::{mono16, table};
use super::*;
use crate::iff::test_build::{Form, fmt_pcm};

fn target(container: OutContainer, frames_out: u64, trim_frames: u64) -> Target {
    Target {
        container,
        channels: 1,
        sample_rate: 44_100,
        bits: 16,
        frames_out,
        trim_frames,
        trim_requested_frames: trim_frames,
        float_source: false,
        bext_update: BextUpdate::Keep,
        tag_edits: Vec::new(),
    }
}

fn plan_of(bytes: &[u8], target: &Target) -> Result<Layout> {
    let (t, f) = table(bytes);
    plan(&mut Cursor::new(bytes), Path::new("t"), &t, &f, target)
}

#[test]
fn an_output_past_4_gib_is_not_dj_safe() {
    let wav = mono16(44_100, &[0; 4]);
    let (mut t, f) = table(&wav);
    // A carried chunk of 2 GiB next to 2 GiB of audio.
    let mut big = t.chunks[0].clone();
    big.id = *b"xbig";
    big.payload = 0..(2 << 30);
    t.chunks.push(big);
    let target = target(OutContainer::RiffWave, 1 << 30, 0);
    let err =
        plan(&mut Cursor::new(&wav), Path::new("t"), &t, &f, &target).expect_err("past 4 GiB");
    assert!(matches!(err, Error::NotDjSafe { .. }), "{err}");
}

#[test]
fn an_aiff_past_2_gib_is_not_dj_safe() {
    let aiff = Form::aiff()
        .chunk(
            b"COMM",
            &crate::iff::test_build::comm(1, 4, 16, 44_100, None),
        )
        .chunk(b"SSND", &crate::iff::test_build::ssnd(0, &[0; 8]))
        .build();
    let (t, f) = table(&aiff);
    // 2^30 frames of 16-bit mono is 2 GiB of audio: fine in a RIFF, too much for AIFF's
    // signed size.
    // The form holds 4 (type) + 26 (COMM) + 16 (SSND header and fields) bytes besides the
    // audio; sizes are even, so the largest form is MAX_AIFF_FORM_SIZE - 1.
    let at_limit = (MAX_AIFF_FORM_SIZE - 46) / 2;
    for (frames, ok) in [(at_limit, true), (at_limit + 1, false), (1 << 30, false)] {
        let target = target(OutContainer::FormAiff, frames, 0);
        let got = plan(&mut Cursor::new(&aiff), Path::new("t"), &t, &f, &target);
        match (got, ok) {
            (Ok(layout), true) => {
                assert_eq!(u64::from(layout.form_size), MAX_AIFF_FORM_SIZE - 1);
            }
            (Err(Error::NotDjSafe { reason, .. }), false) => assert!(reason.contains("2 GiB")),
            (other, _) => panic!("{frames} frames: {other:?}"),
        }
    }
    let wav = mono16(44_100, &[0; 4]);
    plan_of(&wav, &target(OutContainer::RiffWave, 1 << 30, 0)).expect("2 GiB fits a RIFF");
}

#[test]
fn patched_chunks_are_held_up_to_16_mib_together() {
    // fact chunks are always rewritten: four of 4 MiB fit the budget exactly, a fifth does not.
    let big = vec![0_u8; usize::try_from(MAX_PATCHED_CHUNK_BYTES).expect("small")];
    let build = |n: usize| {
        let mut form = Form::riff().chunk(b"fmt ", &fmt_pcm(1, 44_100, 16));
        for _ in 0..n {
            form = form.chunk(b"fact", &big);
        }
        form.chunk(b"data", &[0; 8]).build()
    };
    let t = target(OutContainer::RiffWave, 4, 0);
    let layout = plan_of(&build(4), &t).expect("16 MiB fit");
    assert_eq!(
        layout
            .records
            .iter()
            .filter(|r| r.fate == BlockFate::Patched)
            .count(),
        4
    );
    match plan_of(&build(5), &t) {
        Err(Error::UnsupportedFormat { detail, .. }) => assert!(detail.contains("together")),
        other => panic!("{other:?}"),
    }
    let mut over = big.clone();
    over.push(0);
    let one = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk(b"fact", &over)
        .chunk(b"data", &[0; 8])
        .build();
    assert!(matches!(
        plan_of(&one, &t),
        Err(Error::UnsupportedFormat { .. })
    ));
}

fn smpl(start: u32, end: u32) -> Vec<u8> {
    let mut p = vec![0; 28];
    p.extend_from_slice(&1_u32.to_le_bytes());
    p.extend_from_slice(&0_u32.to_le_bytes());
    p.extend_from_slice(&[0; 8]);
    p.extend_from_slice(&start.to_le_bytes());
    p.extend_from_slice(&end.to_le_bytes());
    p.extend_from_slice(&[0; 8]);
    p
}

#[test]
fn a_trim_into_a_sampler_loop_is_refused_even_when_the_loop_ends_later() {
    let wav = |start, end| {
        Form::riff()
            .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
            .chunk(b"smpl", &smpl(start, end))
            .chunk(b"data", &[0; 2000])
            .build()
    };
    let t = target(OutContainer::RiffWave, 559, 441);
    match plan_of(&wav(100, 10_000), &t) {
        Err(Error::InvalidArgument(m)) => assert!(m.contains("100 to 10000"), "{m}"),
        other => panic!("{other:?}"),
    }
    let layout = plan_of(&wav(441, 10_000), &t).expect("the loop starts at the cut");
    assert_eq!(layout.records[1].fate, BlockFate::Patched);
}

fn bext_wav(bext: &[u8]) -> Vec<u8> {
    Form::riff()
        .chunk(b"bext", bext)
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk(b"data", &[0; 8])
        .build()
}

#[test]
fn bext_fates_follow_version_and_length() {
    let full = |version: u16| {
        let mut p = vec![0; 602];
        p[patch::BEXT_VERSION..patch::BEXT_VERSION + 2].copy_from_slice(&version.to_le_bytes());
        p
    };
    let gain_only = Target {
        bext_update: BextUpdate::ClearIfVersion2,
        ..target(OutContainer::RiffWave, 4, 0)
    };
    let fate = |bytes: &[u8], t: &Target| plan_of(&bext_wav(bytes), t).map(|l| l.records[0].fate);
    assert_eq!(fate(&full(2), &gain_only).ok(), Some(BlockFate::Patched));
    assert_eq!(fate(&full(1), &gain_only).ok(), Some(BlockFate::Carried));
    assert_eq!(fate(&full(0), &gain_only).ok(), Some(BlockFate::Carried));
    // A short bext is carried as long as nothing must change, refused when something must.
    let short = vec![0; 100];
    assert_eq!(fate(&short, &gain_only).ok(), Some(BlockFate::Carried));
    let loud = Target {
        bext_update: BextUpdate::Set([0; 10]),
        ..gain_only.clone()
    };
    assert!(matches!(fate(&short, &loud), Err(Error::Corrupt { .. })));
    let trim = target(OutContainer::RiffWave, 3, 1);
    assert!(matches!(fate(&short, &trim), Err(Error::Corrupt { .. })));
}
