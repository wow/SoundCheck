//! Hand-written expected values for the patched blocks, so the patch rules in `expect.rs` (which
//! the reference writer also uses) are pinned by numbers worked out independently: a 441-sample
//! head trim with the grid's loudness, and gain without loudness. Field readers here are local
//! and read the layouts directly.

use super::apply::{self, ApplyArgs, IDENTITY};
use super::cases::fixture;
use super::expect::{patched, refusal, tags_not_added};
use super::parse::{Kind, parse};

const TRIM: ApplyArgs = ApplyArgs {
    trim_samples: 441,
    loudness: Some(apply::LOUDNESS),
    ..IDENTITY
};

fn le32(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(p[at..at + 4].try_into().expect("4 bytes"))
}

fn be32(p: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"))
}

fn be64(p: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(p[at..at + 8].try_into().expect("8 bytes"))
}

/// Input payload and patched payload of the first block called `id` in fixture `name`.
fn patch(name: &str, id: &str, args: &ApplyArgs) -> (Vec<u8>, Vec<u8>) {
    let fx = fixture(name);
    let input = parse(&fx.bytes).expect("parse");
    let block = input.blocks.iter().find(|b| b.id == id).expect("block");
    let out = patched(id, &block.bytes, &fx, args).expect("patch");
    (block.bytes.clone(), out)
}

/// Asserts `out` equals `input` except in the byte ranges `changed`.
fn same_elsewhere(input: &[u8], out: &[u8], changed: &[std::ops::Range<usize>]) {
    assert_eq!(input.len(), out.len());
    for (i, (a, b)) in input.iter().zip(out).enumerate() {
        if !changed.iter().any(|r| r.contains(&i)) {
            assert_eq!(a, b, "byte {i} changed");
        }
    }
}

#[test]
fn riff_and_aiff_positions_after_a_441_sample_trim() {
    let w24 = "wav24-bwf-ixml-cue-smpl-id3v24";
    let (_, cue) = patch(w24, "cue ", &TRIM);
    let positions: Vec<(u32, u32)> = (0..3)
        .map(|i| (le32(&cue, 4 + 24 * i + 4), le32(&cue, 4 + 24 * i + 20)))
        .collect();
    assert_eq!(
        positions,
        [(0, 0), (0, 0), (10_584, 10_584)],
        "cue 0/300/11025"
    );
    let (_, smpl) = patch(w24, "smpl", &TRIM);
    assert_eq!(
        (le32(&smpl, 44), le32(&smpl, 48)),
        (559, 10_584),
        "loop 1000..11025"
    );
    let (_, mark) = patch("aiff24-48k-mark", "MARK", &TRIM);
    let mut at = 2;
    let mut marks = Vec::new();
    for _ in 0..3 {
        marks.push(be32(&mark, at + 2));
        at += 6 + (1 + usize::from(mark[at + 6])).next_multiple_of(2);
    }
    assert_eq!(marks, [0, 0, 11_559], "markers 0/300/12000");
    let (_, fact) = patch("wav24-extensible-bext-v1-fact", "fact", &TRIM);
    assert_eq!(le32(&fact, 0), 19_559);
}

/// (track number, track offset, [(index number, relative offset)]).
type Track = (u8, u64, Vec<(u8, u64)>);

fn tracks(p: &[u8]) -> Vec<Track> {
    let mut at = 396;
    let mut out = Vec::new();
    for _ in 0..p[395] {
        let count = usize::from(p[at + 35]);
        let indices = (0..count)
            .map(|k| (p[at + 36 + 12 * k + 8], be64(p, at + 36 + 12 * k)))
            .collect();
        out.push((p[at + 8], be64(p, at), indices));
        at += 36 + 12 * count;
    }
    out
}

#[test]
fn cuesheet_index_points_shift_and_clamp_one_by_one() {
    let (input, out) = patch("flac16-all-blocks", "CUESHEET", &TRIM);
    let before: Vec<Track> = vec![
        (1, 0, vec![(0, 0), (1, 588)]),
        (2, 2205, vec![(0, 0), (1, 588)]),
        (255, 20_000, vec![]),
    ];
    assert_eq!(tracks(&input), before);
    // Track 1: index 00 at 0 clamps to 0, index 01 moves from 588 to 147 (absolute), so the
    // pregap shrinks; track 2 starts at 2205 - 441 = 1764 with its indices unchanged relative
    // to it; the lead-out is the new total.
    let after: Vec<Track> = vec![
        (1, 0, vec![(0, 0), (1, 147)]),
        (2, 1764, vec![(0, 0), (1, 588)]),
        (255, 19_559, vec![]),
    ];
    assert_eq!(tracks(&out), after);
}

#[test]
fn bext_time_reference_version_and_loudness_are_literal() {
    for (name, time) in [
        ("wav24-bwf-ixml-cue-smpl-id3v24", 158_760_441_u64),
        ("rf64-24-48k", 172_800_441),
        ("wav16-mono-bext-v0", 158_760_441),
    ] {
        let (input, out) = patch(name, "bext", &TRIM);
        let reference = u64::from_le_bytes(out[338..346].try_into().expect("8 bytes"));
        assert_eq!(reference, time, "{name} TimeReference");
        assert_eq!(out[346..348], [2, 0], "{name} Version");
        let loudness = [0x9D, 0xFB, 0xFF, 0x7F, 0xCC, 0xFF, 0xD4, 0xFC, 0x77, 0xFC];
        assert_eq!(out[412..422], loudness, "{name} loudness");
        same_elsewhere(&input, &out, &[338..348, 412..422]);
    }
    // Gain without new loudness: every field becomes "not measured", never the stale values.
    let gain = ApplyArgs {
        gain_db: -3.2,
        ..IDENTITY
    };
    let (input, out) = patch("wav24-bwf-ixml-cue-smpl-id3v24", "bext", &gain);
    assert_eq!(out[346..348], [2, 0]);
    assert_eq!(out[412..422], [0xFF, 0x7F].repeat(5)[..]);
    same_elsewhere(&input, &out, &[346..348, 412..422]);
    // Gain without loudness on version 0 and 1: they hold no loudness, so nothing changes.
    for name in ["wav16-mono-bext-v0", "wav24-extensible-bext-v1-fact"] {
        let (input, out) = patch(name, "bext", &gain);
        assert_eq!(input, out, "{name}");
    }
    // Neither gain nor trim nor loudness: unchanged.
    let (input, out) = patch("wav24-bwf-ixml-cue-smpl-id3v24", "bext", &IDENTITY);
    assert_eq!(input, out);
}

#[test]
fn tags_are_not_added_to_files_readers_disagree_on_or_that_a_crc_protects() {
    for fx in super::cases::matrix() {
        let parsed = parse(&fx.bytes).expect("parse");
        let want = match fx.name {
            "wav16-mono-two-id3-chunks-ext-headers" => Some("two ID3 tags"),
            "wav16-mono-id3v24-tag-unsync" => Some("tag-level unsynchronisation"),
            "wav16-mono-id3v24-ext-crc" => Some("extended header with a CRC"),
            _ => None,
        };
        assert_eq!(tags_not_added(&parsed), want, "{}", fx.name);
    }
    // The CRC in the fixture is the CRC-32 of frames and padding, stored as 35-bit syncsafe.
    let fx = fixture("wav16-mono-id3v24-ext-crc");
    let parsed = parse(&fx.bytes).expect("parse");
    let ext = parsed
        .blocks
        .iter()
        .find(|b| b.kind == Kind::Id3ExtHeader)
        .expect("extended header");
    assert_eq!(
        ext.bytes[..7],
        [0, 0, 0, 12, 1, 0x20, 5],
        "size, flags, CRC data length"
    );
    let stored = ext.bytes[7..12]
        .iter()
        .fold(0_u64, |acc, b| (acc << 7) | u64::from(*b));
    let tag = &parsed.find(Kind::Chunk, "ID3 ").expect("tag").bytes;
    let data = &tag[10 + ext.bytes.len()..];
    assert_eq!(stored, u64::from(super::id3::crc32(data)));
}

#[test]
fn refusals_name_their_error_class() {
    let partly_cut = fixture("wav16-mono-smpl-loop-in-head");
    let kind = |name: &str, args: &ApplyArgs| refusal(&fixture(name), args).map(|r| r.kind);
    let trim = ApplyArgs {
        trim_samples: 441,
        ..IDENTITY
    };
    // The loop runs from 100 to 10,000: it ends long after the trim but starts inside it.
    assert_eq!(
        refusal(&partly_cut, &trim).map(|r| r.kind).as_deref(),
        Some("invalidArgument")
    );
    assert_eq!(refusal(&partly_cut, &IDENTITY), None);
    // The loop from 1,000 lies after the trim.
    assert_eq!(kind("wav24-bwf-ixml-cue-smpl-id3v24", &trim), None);
    let hot = "wav-float32-mono-over-full-scale";
    assert_eq!(kind(hot, &IDENTITY).as_deref(), Some("wouldClip"));
    let cut = ApplyArgs {
        gain_db: -3.2,
        ..IDENTITY
    };
    assert_eq!(kind(hot, &cut), None);
}
