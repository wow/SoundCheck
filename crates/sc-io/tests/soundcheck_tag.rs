//! The `SOUNDCHECK` record survives the trip through the renderers: written as a tag edit by
//! `apply_iff` into an `ID3v2`.3 tag (UTF-16, replacing an older UTF-16 record) and an `ID3v2`.4
//! tag (UTF-8), and by `apply_flac` into a Vorbis comment, then read back by `tags::scan`
//! exactly as it was written.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::export::{BatchMode, RecordGain, SoundcheckRecord};
use sc_core::plan::{Codec, LoudnessMode};
use sc_core::{Bpm, Lufs, RenderRequest, SampleIndex, TagEdit};
use sc_io::tags::scan;
use sc_io::{apply_flac, apply_iff};

/// A record whose version holds a non-ASCII character, so ID3 writes it as UTF-16 in v2.3 and
/// UTF-8 in v2.4.
fn record() -> SoundcheckRecord {
    SoundcheckRecord {
        app: "0.1.0-ß".into(),
        mode: BatchMode::Prepare,
        gain: Some(RecordGain {
            stat: LoudnessMode::Dj,
            target: Lufs(-11.0),
            gain_db: -1.25,
        }),
        trim_frames: 0,
        sample_rate: 44_100,
        bpm: Some(Bpm(127.98)),
        bar1: Some(SampleIndex(221)),
        grid_withheld: false,
        source_hash: Some([0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]),
    }
}

/// UTF-16 text with a byte-order mark (little-endian), optionally terminated.
fn utf16(text: &str, terminated: bool) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    if terminated {
        out.extend_from_slice(&[0, 0]);
    }
    out
}

/// An ID3 tag of `major` holding `frames` (id, body) and 512 bytes of padding.
fn id3(major: u8, frames: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, b) in frames {
        let len = u32::try_from(b.len()).expect("small");
        body.extend_from_slice(*id);
        if major == 3 {
            body.extend_from_slice(&len.to_be_bytes());
        } else {
            let s = |shift: u32| u8::try_from((len >> shift) & 0x7F).expect("7 bits");
            body.extend_from_slice(&[s(21), s(14), s(7), s(0)]);
        }
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(b);
    }
    body.resize(body.len() + 512, 0);
    let size = u32::try_from(body.len()).expect("small");
    let s = |shift: u32| u8::try_from((size >> shift) & 0x7F).expect("7 bits");
    let mut tag = vec![b'I', b'D', b'3', major, 0, 0, s(21), s(14), s(7), s(0)];
    tag.extend_from_slice(&body);
    tag
}

/// `file` (RIFF or FORM) with a chunk appended and the container size updated.
fn with_chunk(mut file: Vec<u8>, id: [u8; 4], payload: &[u8], big_endian: bool) -> Vec<u8> {
    let len = u32::try_from(payload.len()).expect("small");
    file.extend_from_slice(&id);
    file.extend_from_slice(&if big_endian {
        len.to_be_bytes()
    } else {
        len.to_le_bytes()
    });
    file.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        file.push(0);
    }
    let size = u32::try_from(file.len() - 8).expect("small");
    file[4..8].copy_from_slice(&if big_endian {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
    file
}

fn render(dir: &Path, name: &str, bytes: &[u8], label: &str) -> PathBuf {
    let input = dir.join(name);
    std::fs::write(&input, bytes).expect("written");
    let output = dir.join(format!("out-{name}"));
    let req = RenderRequest {
        gain_db: -1.25,
        tag_edits: vec![TagEdit {
            label: label.into(),
            value: record().to_value(),
        }],
        ..RenderRequest::default()
    };
    let cancel = AtomicBool::new(false);
    let report = if name.ends_with("flac") {
        apply_flac(&input, &output, &req, &cancel)
    } else {
        apply_iff(&input, &output, &req, &cancel)
    }
    .expect("rendered");
    assert!(report.tags_added, "{name}: {:?}", report.tags_not_added);
    output
}

fn read_back(path: &Path, codec: Codec) {
    let found = scan(path, codec);
    assert_eq!(found.soundcheck, Some(record()), "{}", path.display());
    assert_eq!(found.soundcheck_unreadable, None);
    assert!(!found.serato_unknown && found.serato.is_empty());
}

#[test]
fn the_record_written_by_a_render_reads_back() {
    let dir = tempfile::tempdir().expect("temp dir");
    // v2.3 WAV holding an older record in UTF-16, which the render replaces in place.
    let mut old = vec![1];
    old.extend(utf16("SOUNDCHECK", true));
    old.extend(utf16(
        "v=1;app=0.0.1;mode=library;gain=none;trim=0;rate=44100",
        false,
    ));
    let mut title = vec![1];
    title.extend(utf16("Título", false));
    let v23 = id3(3, &[(b"TIT2", title), (b"TXXX", old)]);
    let wav = with_chunk(common::wav(4_000, 3), *b"id3 ", &v23, false);
    let out = render(dir.path(), "v23.wav", &wav, "TXXX:SOUNDCHECK");
    read_back(&out, Codec::Wav);
    let bytes = std::fs::read(&out).expect("output");
    let marked = utf16("0.1.0-ß", false);
    assert!(
        bytes.windows(marked.len() - 2).any(|w| w == &marked[2..]),
        "written as UTF-16"
    );
    // v2.4 AIFF: the record is appended in UTF-8.
    let v24 = id3(4, &[(b"TIT2", b"\x03Title".to_vec())]);
    let aiff = with_chunk(common::aiff(4_000, 5), *b"ID3 ", &v24, true);
    let out = render(dir.path(), "v24.aiff", &aiff, "TXXX:SOUNDCHECK");
    read_back(&out, Codec::Aiff);
    assert!(
        std::fs::read(&out)
            .expect("output")
            .windows(8)
            .any(|w| w == "0.1.0-ß".as_bytes()),
        "written as UTF-8"
    );
    // FLAC: a Vorbis comment field.
    let out = render(dir.path(), "t.flac", &common::flac(4_000, 7), "SOUNDCHECK");
    read_back(&out, Codec::Flac);
}
