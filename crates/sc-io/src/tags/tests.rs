//! Unit tests of `crates/sc-io/src/tags.rs` and its parts: Serato data and the `SOUNDCHECK`
//! record found in WAV, AIFF, MP3 and FLAC tags, and the loudness items a gain change reports
//! as stale. Every file is built here; no tagged audio from a real library is used.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::export::{BatchMode, SoundcheckRecord};
use sc_core::ipc::SeratoTag;
use sc_core::plan::Codec;
use sc_core::{RenderRequest, TagEdit};

use super::*;
use crate::flac::test_build::{encode, file as flac_file, samples, vorbis};
use crate::id3::test_build::{comm, frame, geob, tag, text, txxx};
use crate::iff::test_build::{Form, comm as aiff_comm, fmt_pcm, ssnd};
use crate::render::{apply_flac, apply_iff};

const RECORD: &str = "v=1;app=0.1.0;mode=library;stat=S-P95;target=-11.00;gain=-1.50;\
                      trim=0;rate=44100;bpm=128.00;bar1=210;src=0011223344556677";

/// A stereo 16-bit 44.1 kHz WAV of 100 frames with `id3` as its `id3 ` chunk.
fn wav(id3: &[u8]) -> Vec<u8> {
    Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &[1; 400])
        .chunk(b"id3 ", id3)
        .build()
}

/// The same as an AIFF with `id3` as its `ID3 ` chunk.
fn aiff(id3: &[u8]) -> Vec<u8> {
    Form::aiff()
        .chunk(b"COMM", &aiff_comm(2, 100, 16, 44_100, None))
        .chunk(b"SSND", &ssnd(0, &[1; 400]))
        .chunk(b"ID3 ", id3)
        .build()
}

/// A mono 16-bit FLAC of 5,000 frames with one Vorbis comment of `fields`.
fn flac(fields: &[&str]) -> Vec<u8> {
    let (frames, info) = encode(&samples(5000, 1, 16, 7), 44_100, 1, 16);
    flac_file(&[], &info, &[(4, vorbis("v", fields))], &frames, &[])
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("written");
    path
}

/// A tag shaped like one Serato writes, with our record.
fn serato_tag(major: u8) -> Vec<u8> {
    tag(
        major,
        0,
        &[],
        &[
            text(major, *b"TIT2", 0, "Matrix Tone"),
            geob(
                major,
                "Serato Autotags",
                b"\x01\x01128.00\0-3.257\0-0.000\0",
            ),
            geob(major, "Serato Markers2", b"\x01\x01AQFDT0xPUgAAAAAEAP///w"),
            geob(major, "Serato BeatGrid", &[1, 0, 0, 0, 0, 1]),
            geob(major, "Serato Overview", &[1, 5]),
            txxx(major, 0, "SOUNDCHECK", RECORD),
        ],
        64,
    )
}

#[test]
fn serato_geob_detected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let expected = [
        SeratoTag::Markers,
        SeratoTag::BeatGrid,
        SeratoTag::Autotags,
        SeratoTag::Overview,
    ];
    let record = SoundcheckRecord::parse(RECORD).expect("record");
    for (name, bytes, codec) in [
        ("v23.wav", wav(&serato_tag(3)), Codec::Wav),
        ("v24.aiff", aiff(&serato_tag(4)), Codec::Aiff),
    ] {
        let path = write(dir.path(), name, &bytes);
        let found = scan(&path, codec);
        assert_eq!(found.serato, expected, "{name}");
        assert_eq!(found.soundcheck.as_ref(), Some(&record), "{name}");
        assert_eq!(found.loudness, ["GEOB:Serato Autotags"], "{name}");
        // The probe that fills a row reads the same.
        let info = crate::probe(&path);
        assert!(info.serato, "{name}");
        assert_eq!(info.serato_tags, expected, "{name}");
        assert_eq!(info.soundcheck.as_ref(), Some(&record), "{name}");
    }
    // MP3: the tag at the start of the file (the frames after it are not read).
    let mut mp3 = serato_tag(3);
    mp3.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64, 0, 0, 0, 0]);
    let path = write(dir.path(), "t.mp3", &mp3);
    let found = scan(&path, Codec::Mp3);
    assert_eq!(found.serato, expected);
    assert_eq!(found.soundcheck.map(|r| r.mode), Some(BatchMode::Library));
}

#[test]
fn serato_objects_in_a_tag_the_index_refuses_still_count() {
    let dir = tempfile::tempdir().expect("temp dir");
    // Tag-level unsynchronisation (header flag 0x80) is refused by the index.
    let mut unsynced = tag(3, 0, &[], &[geob(3, "Serato Markers2", &[1, 1])], 16);
    unsynced[5] = 0x80;
    let path = write(dir.path(), "u.wav", &wav(&unsynced));
    assert!(crate::id3::parse_tag(&unsynced).is_err());
    assert_eq!(scan(&path, Codec::Wav).serato, [SeratoTag::Markers]);
    // ID3v2.2 (three-letter frame ids, `GEO`).
    let mut v22 = tag(3, 0, &[], &[], 0);
    v22[3] = 2;
    v22.extend_from_slice(b"GEO\0\0\x20\0application/octet-stream\0\0Serato BeatGrid\0\x01");
    let path = write(dir.path(), "v22.aiff", &aiff(&v22));
    assert_eq!(scan(&path, Codec::Aiff).serato, [SeratoTag::BeatGrid]);
}

#[test]
fn serato_vorbis_detected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let soundcheck = format!("soundcheck={RECORD}");
    let path = write(
        dir.path(),
        "s.flac",
        &flac(&[
            "TITLE=x",
            "SERATO_MARKERS_V2=AQFDT0xPUgAAAAAEAP",
            "SERATO_BEATGRID=AQAAAAAB",
            "serato_autogain=AQExMjguMDAA",
            "SERATO_RELVOL=AQE",
            &soundcheck,
        ]),
    );
    let found = scan(&path, Codec::Flac);
    assert_eq!(
        found.serato,
        [
            SeratoTag::Markers,
            SeratoTag::BeatGrid,
            SeratoTag::Autotags,
            SeratoTag::Other
        ]
    );
    assert_eq!(found.soundcheck, SoundcheckRecord::parse(RECORD).ok());
    assert_eq!(found.loudness, ["serato_autogain"]);
    let info = crate::probe(&path);
    assert!(info.serato && info.soundcheck.is_some());
}

#[test]
fn files_without_serato_data_or_a_readable_record() {
    let dir = tempfile::tempdir().expect("temp dir");
    let plain = tag(
        4,
        0,
        &[],
        &[
            text(4, *b"TIT2", 0, "A"),
            geob(4, "Traktor4", &[0]),
            // A record of a later format is not read.
            txxx(4, 0, "SOUNDCHECK", "v=2;app=9.0.0"),
        ],
        16,
    );
    let path = write(dir.path(), "plain.wav", &wav(&plain));
    assert_eq!(scan(&path, Codec::Wav), TagScan::default());
    let info = crate::probe(&path);
    assert!(!info.serato && info.serato_tags.is_empty() && info.soundcheck.is_none());
    let path = write(dir.path(), "plain.flac", &flac(&["TITLE=y"]));
    assert_eq!(scan(&path, Codec::Flac), TagScan::default());
    // A file that is not what its codec says, or is missing, reads as nothing.
    assert_eq!(scan(&path, Codec::Wav), TagScan::default());
    assert_eq!(
        scan(&dir.path().join("gone.wav"), Codec::Wav),
        TagScan::default()
    );
    assert_eq!(scan(&path, Codec::Aac), TagScan::default());
}

#[test]
fn loudness_items_are_classified() {
    let t = tag(
        3,
        0,
        &[],
        &[
            txxx(3, 0, "replaygain_album_gain", "-1 dB"),
            txxx(3, 0, "REPLAYGAIN_TRACK_PEAK", "0.9"),
            txxx(3, 0, "iTunNORM", "not where iTunes writes it"),
            comm(3, 0, "iTunNORM", " 00000A2B"),
            comm(3, 1, "iTunSMPB", " 00000000 00000210"),
            comm(3, 0, "", "a comment"),
            frame(3, *b"RVA2", [0, 0], b"track\0\x01\x00\x10\x00"),
            geob(3, "Serato Autotags", &[1, 1]),
            geob(3, "Serato Markers2", &[1, 1]),
        ],
        0,
    );
    let index = crate::id3::parse_tag(&t).expect("parses");
    let labels: Vec<String> = index.frames.iter().filter_map(id3_loudness_label).collect();
    assert_eq!(
        labels,
        [
            "TXXX:replaygain_album_gain",
            "TXXX:REPLAYGAIN_TRACK_PEAK",
            "COMM:iTunNORM",
            "RVA2",
            "GEOB:Serato Autotags"
        ]
    );
    for name in [
        "REPLAYGAIN_TRACK_GAIN",
        "r128_track_gain",
        "ITUNNORM",
        "SERATO_AUTOGAIN",
        "serato_autotags",
    ] {
        assert!(is_vorbis_loudness(name.as_bytes()), "{name}");
    }
    for name in ["SERATO_BEATGRID", "ITUNSMPB", "REPLAYGAIN", "TITLE", "R128"] {
        assert!(!is_vorbis_loudness(name.as_bytes()), "{name}");
    }
}

fn gain(db: f64, edits: &[(&str, &str)]) -> RenderRequest {
    RenderRequest {
        gain_db: db,
        tag_edits: edits
            .iter()
            .map(|(l, v)| TagEdit {
                label: (*l).into(),
                value: (*v).into(),
            })
            .collect(),
        ..RenderRequest::default()
    }
}

/// The stale loudness tags of rendering `bytes` (named `name`) with `req`.
fn stale_after(name: &str, bytes: &[u8], req: &RenderRequest) -> Vec<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = write(dir.path(), name, bytes);
    let output = dir.path().join(format!("out-{name}"));
    let cancel = AtomicBool::new(false);
    let report = if input.extension().is_some_and(|e| e == "flac") {
        apply_flac(&input, &output, req, &cancel)
    } else {
        apply_iff(&input, &output, req, &cancel)
    };
    report.expect("rendered").stale_loudness_tags
}

#[test]
fn stale_itunnorm_reported() {
    let t = tag(
        3,
        0,
        &[],
        &[
            comm(3, 0, "iTunNORM", " 00000A2B 00000A2B 00004A1C"),
            comm(3, 0, "iTunSMPB", " 00000000 00000210"),
            txxx(3, 0, "REPLAYGAIN_TRACK_GAIN", "-4.00 dB"),
        ],
        256,
    );
    let ours = [("TXXX:REPLAYGAIN_TRACK_GAIN", "-2.50 dB")];
    assert_eq!(
        stale_after("n.wav", &wav(&t), &gain(-1.5, &ours)),
        ["COMM:iTunNORM"]
    );
    assert_eq!(
        stale_after("n.aiff", &aiff(&t), &gain(-1.5, &[])),
        ["COMM:iTunNORM", "TXXX:REPLAYGAIN_TRACK_GAIN"]
    );
    assert_eq!(
        stale_after("n.wav", &wav(&t), &gain(0.0, &ours)),
        Vec::<String>::new()
    );
    let f = flac(&["ITUNNORM= 00000A2B", "ITUNSMPB= 0"]);
    assert_eq!(stale_after("n.flac", &f, &gain(-1.5, &[])), ["ITUNNORM"]);
}

#[test]
fn stale_serato_autotags_reported() {
    assert_eq!(
        stale_after("s.wav", &wav(&serato_tag(3)), &gain(-2.0, &[])),
        ["GEOB:Serato Autotags"]
    );
    assert_eq!(
        stale_after("s.aiff", &aiff(&serato_tag(4)), &gain(1.0, &[])),
        ["GEOB:Serato Autotags"]
    );
    let f = flac(&[
        "SERATO_AUTOGAIN=AQE",
        "SERATO_AUTOTAGS=AQE",
        "SERATO_MARKERS_V2=AQE",
    ]);
    assert_eq!(
        stale_after("s.flac", &f, &gain(-2.0, &[])),
        ["SERATO_AUTOGAIN", "SERATO_AUTOTAGS"]
    );
    assert_eq!(
        stale_after("s.flac", &f, &gain(0.0, &[])),
        Vec::<String>::new()
    );
}
