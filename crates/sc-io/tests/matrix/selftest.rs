//! Self-tests of the checks: the reference writer (`oracle.rs`) passes every fixture and row
//! through the same runner the writer tests use, each single injected mistake is rejected, and
//! the low-level encodings round-trip against hand-written bytes.

use super::aiff;
use super::apply::{self, Applied, ApplyArgs, IDENTITY, TagEdit, grid};
use super::cases::{Fixture, fixture, matrix};
use super::check::check_output;
use super::flac;
use super::id3;
use super::inspect;
use super::oracle::{self, Options, ideal_samples};
use super::parse::{self, Container, Kind, parse};
use super::writers::{Written, bit_depth_rows, has_id3, run, tag_rows};

fn reference(opts: Options) -> impl Fn(&Fixture, &ApplyArgs, &[TagEdit]) -> Written {
    move |fx, args, edits| oracle::write(fx, args, edits, &opts)
}

fn passes(fixtures: &[Fixture], rows: &[ApplyArgs], edits: &[TagEdit]) {
    let writer = reference(Options::default());
    for fx in fixtures {
        for args in rows {
            run(&writer, fx, args, edits).unwrap_or_else(|e| panic!("{e}"));
        }
    }
}

fn of(containers: &[Container]) -> Vec<Fixture> {
    matrix()
        .into_iter()
        .filter(|f| containers.contains(&f.container))
        .collect()
}

#[test]
fn reference_writer_passes_every_wav_row() {
    passes(&of(&[Container::Wave, Container::Rf64]), &grid(), &[]);
}

#[test]
fn reference_writer_passes_every_aiff_row() {
    passes(&of(&[Container::Aiff, Container::Aifc]), &grid(), &[]);
}

#[test]
fn reference_writer_passes_every_flac_row_with_vorbis_edits() {
    passes(&of(&[Container::Flac]), &grid(), &apply::vorbis_edits());
}

#[test]
fn reference_writer_passes_bit_depth_and_id3_rows() {
    let iff: Vec<_> = matrix().into_iter().filter(Fixture::is_iff).collect();
    passes(&iff, &bit_depth_rows(), &[]);
    let tagged: Vec<_> = matrix().into_iter().filter(has_id3).collect();
    passes(&tagged, &tag_rows(), &apply::id3_edits());
}

/// Asserts that the reference writer with `opts` fails the checks on `name` with `args`.
fn rejects(name: &str, args: &ApplyArgs, edits: &[TagEdit], opts: Options, what: &str) {
    let fx = fixture(name);
    match run(&reference(opts), &fx, args, edits) {
        Ok(()) => panic!("{what} on {name} went unnoticed"),
        Err(e) => eprintln!("{what}: rejected with: {e}"),
    }
}

const GAIN: ApplyArgs = ApplyArgs {
    gain_db: -3.2,
    ..IDENTITY
};
const TRIM: ApplyArgs = ApplyArgs {
    trim_samples: 441,
    ..IDENTITY
};
const LOUD: ApplyArgs = ApplyArgs {
    loudness: Some(apply::LOUDNESS),
    ..IDENTITY
};

fn samples(name: &str, args: &ApplyArgs, f: impl Fn(usize, i32, f64) -> i32) -> Options {
    let fx = fixture(name);
    let ideal = ideal_samples(&fx, args);
    let gain = 10_f64.powf(args.gain_db / 20.0);
    let scale = f64::from(1_u32 << (super::expect::out_bits(&fx, args) - 1));
    let altered = ideal
        .iter()
        .enumerate()
        .map(|(i, y)| f(i, *y, fx.source.normalised(i) * gain * scale))
        .collect();
    Options {
        samples: Some(altered),
        ..Options::default()
    }
}

#[test]
fn pcm_checks_reject_floor_offset_missing_dither_and_inexact_widening() {
    let w24 = "wav24-bwf-ixml-cue-smpl-id3v24";
    // Truncation is exact float-to-integer here: the values are within the 24-bit range.
    #[allow(clippy::cast_possible_truncation)]
    let floor = samples(w24, &GAIN, |_, _, r| r.floor() as i32);
    rejects(w24, &GAIN, &[], floor, "floor instead of round");
    rejects(
        w24,
        &GAIN,
        &[],
        samples(w24, &GAIN, |_, y, _| y + 1),
        "+1 LSB everywhere",
    );
    let widen = ApplyArgs {
        bits: Some(24),
        ..IDENTITY
    };
    let inexact = samples("wav16-mono", &widen, |i, y, _| y + i32::from(i % 97 == 0));
    rejects("wav16-mono", &widen, &[], inexact, "16 -> 24 not exact");
    #[allow(clippy::cast_possible_truncation)]
    let undithered = samples("wav16-mono", &GAIN, |_, _, r| r.round() as i32);
    rejects(
        "wav16-mono",
        &GAIN,
        &[],
        undithered,
        "16-bit gain without dither",
    );
}

fn unpatched(id: &'static str) -> Options {
    Options {
        unpatched: Some(id),
        ..Options::default()
    }
}

#[test]
fn patch_checks_reject_unshifted_positions_stale_lengths_and_missing_loudness() {
    let w24 = "wav24-bwf-ixml-cue-smpl-id3v24";
    rejects(w24, &TRIM, &[], unpatched("cue "), "cue points not shifted");
    rejects(w24, &TRIM, &[], unpatched("smpl"), "loop not shifted");
    rejects(
        w24,
        &TRIM,
        &[],
        unpatched("bext"),
        "TimeReference not moved",
    );
    rejects(w24, &LOUD, &[], unpatched("bext"), "loudness not written");
    rejects(
        "wav16-mono-bext-v0",
        &LOUD,
        &[],
        unpatched("bext"),
        "v0 not upgraded",
    );
    let ext = "wav24-extensible-bext-v1-fact";
    rejects(ext, &TRIM, &[], unpatched("fact"), "stale fact length");
    rejects(
        "aiff24-48k-mark",
        &TRIM,
        &[],
        unpatched("MARK"),
        "markers not shifted",
    );
    let edits = apply::vorbis_edits();
    let cue = unpatched("CUESHEET");
    rejects(
        "flac16-all-blocks",
        &TRIM,
        &edits,
        cue,
        "CUESHEET not shifted",
    );
}

#[test]
fn tag_checks_reject_wrong_encoding_flags_duplicates_and_growth() {
    let id3 = apply::id3_edits();
    let w16 = "wav16-info-id3v23-pad512";
    let utf8 = Options {
        text_encoding: Some(3),
        ..Options::default()
    };
    rejects(w16, &IDENTITY, &id3, utf8.clone(), "UTF-8 in a v2.3 tag");
    let flags = Options {
        frame_flags: [0x40, 0],
        ..Options::default()
    };
    rejects(w16, &IDENTITY, &id3, flags, "frame flags on our frames");
    let dup = Options {
        append_existing: true,
        ..Options::default()
    };
    rejects(
        w16,
        &IDENTITY,
        &id3,
        dup.clone(),
        "existing frames appended again",
    );
    let grow = Options {
        grow_tag: true,
        ..Options::default()
    };
    rejects(
        w16,
        &IDENTITY,
        &id3,
        grow,
        "tag grown although padding fits",
    );
    let flac_edits = apply::vorbis_edits();
    rejects(
        "flac16-all-blocks",
        &IDENTITY,
        &flac_edits,
        dup,
        "Vorbis fields appended again",
    );
    // UTF-8 is a valid encoding in v2.4, so the same option passes there.
    let v24 = fixture("wav24-bwf-ixml-cue-smpl-id3v24");
    run(&reference(utf8), &v24, &IDENTITY, &id3).expect("UTF-8 in v2.4");
    // A writer that reports "tags added" for a tag it must not edit is caught.
    let unsync = fixture("wav16-mono-id3v24-tag-unsync");
    let (bytes, _) = oracle::write(&unsync, &IDENTITY, &id3, &Options::default()).expect("write");
    let wrong = Applied { tags_added: true };
    assert!(check_output(&unsync, &bytes, &IDENTITY, &id3, wrong).is_err());
}

#[test]
fn block_checks_reject_a_changed_chunk_a_changed_pad_byte_and_foreign_headers() {
    let fx = fixture("wav24-bwf-ixml-cue-smpl-id3v24");
    let (good, applied) = oracle::write(&fx, &IDENTITY, &[], &Options::default()).expect("write");
    check_output(&fx, &good, &IDENTITY, &[], applied).expect("reference output");
    let parsed = parse(&good).expect("parse");
    let ixml = parsed.find(Kind::Chunk, "iXML").expect("iXML");
    let mut changed = good.clone();
    changed[ixml.offset + 20] ^= 0x01;
    assert!(check_output(&fx, &changed, &IDENTITY, &[], applied).is_err());
    let odd = parsed.find(Kind::Chunk, "xodd").expect("xodd");
    let mut repadded = good.clone();
    repadded[odd.offset + 8 + odd.bytes.len()] = 0;
    assert!(check_output(&fx, &repadded, &IDENTITY, &[], applied).is_err());
    for name in [
        "wav24-extensible-bext-v1-fact",
        "rf64-24-48k",
        "aifc-sowt-16",
    ] {
        let fx = fixture(name);
        let as_is = check_output(&fx, &fx.bytes, &IDENTITY, &[], applied);
        assert!(
            as_is.is_err(),
            "{name} passed through unchanged went unnoticed"
        );
    }
}

#[test]
fn flac_checks_reject_a_wrong_md5_a_stray_seek_point_and_a_changed_block() {
    let fx = fixture("flac16-all-blocks");
    let none = Applied { tags_added: false };
    check_output(&fx, &fx.bytes, &IDENTITY, &[], none).expect("the input is a valid output");
    let parsed = parse(&fx.bytes).expect("parse");
    let at = |id: &str| parsed.find(Kind::FlacBlock, id).expect("block").offset + 4;
    // STREAMINFO MD5 starts 18 bytes into the payload; seek point 1's offset ends at 18 + 16;
    // PADDING must stay zero.
    let damage = [
        at("STREAMINFO") + 18,
        at("SEEKTABLE") + 18 + 15,
        at("PICTURE") + 40,
        at("PADDING") + 3,
    ];
    for byte in damage {
        let mut damaged = fx.bytes.clone();
        damaged[byte] ^= 0x01;
        let result = check_output(&fx, &damaged, &IDENTITY, &[], none);
        assert!(result.is_err(), "damage at byte {byte} went unnoticed");
    }
}

#[test]
fn aiff_extended_sample_rates_are_exact() {
    let cases: [(u32, [u8; 10]); 4] = [
        (44_100, [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
        (48_000, [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]),
        (88_200, [0x40, 0x0F, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
        (96_000, [0x40, 0x0F, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]),
    ];
    for (rate, bytes) in cases {
        assert_eq!(aiff::extended(rate), bytes, "{rate} Hz");
        let back = inspect::extended_to_f64(&bytes);
        assert!(back.to_bits() == f64::from(rate).to_bits(), "{rate} Hz");
    }
}

#[test]
fn id3_frame_sizes_are_plain_in_v23_and_syncsafe_in_v24() {
    let value = "x".repeat(198); // body: encoding + "D" + NUL + 198 bytes = 201 bytes
    for (version, size) in [
        (id3::Version::V23, [0x00, 0x00, 0x00, 0xC9]),
        (id3::Version::V24, [0x00, 0x00, 0x01, 0x49]),
    ] {
        let tag = id3::tag(version, &[id3::txxx(version, "D", &value)], 3);
        assert_eq!(tag.bytes[14..18], size, "{version:?} frame size");
        assert_eq!(
            tag.bytes[6..10],
            [0, 0, 0x01, 0x56],
            "tag size 211 + 3 padding"
        );
        let mut chunk = b"WAVEid3 ".to_vec();
        chunk.extend_from_slice(&u32::try_from(tag.bytes.len()).expect("small").to_le_bytes());
        chunk.extend_from_slice(&tag.bytes);
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&u32::try_from(chunk.len()).expect("small").to_le_bytes());
        file.extend_from_slice(&chunk);
        let parsed = parse(&file).expect("parse");
        assert_eq!(parsed.tags[0].padding, 3);
        assert_eq!(parsed.blocks[1].id, "TXXX:D");
        let major = parsed.tags[0].version;
        let text = parse::frame_text(major, &parsed.blocks[1]).map(|(_, v)| v);
        assert_eq!(text, Some(value.clone()));
    }
}

#[test]
fn flac_frame_numbers_round_trip_through_the_utf8_coding() {
    let cases: [(u64, &[u8]); 6] = [
        (0x7F, &[0x7F]),
        (0x80, &[0xC2, 0x80]),
        (0x7FF, &[0xDF, 0xBF]),
        (0x800, &[0xE0, 0xA0, 0x80]),
        (0xFFFF, &[0xEF, 0xBF, 0xBF]),
        ((1 << 36) - 1, &[0xFE, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF]),
    ];
    for (n, bytes) in cases {
        assert_eq!(flac::coded_number(n), bytes, "encode {n:#x}");
        assert_eq!(
            inspect::decode_coded_number(bytes),
            Some((n, bytes.len())),
            "decode {n:#x}"
        );
    }
}
