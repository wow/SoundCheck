//! Serato data found in the source file feeds the planner: a Prepare cut in place is refused and
//! a cut copy gets its notice. The files are built here (an `ID3v2.3` tag with `GEOB` objects
//! laid out as Serato writes them); no tagged audio from a real library is used.
use std::path::Path;

use sc_core::ipc::{FileInfo, SeratoTag};

use super::*;

/// A `GEOB` frame (ISO-8859-1, `application/octet-stream`, no file name) described `desc`.
fn geob(desc: &str) -> Vec<u8> {
    let mut body = b"\0application/octet-stream\0\0".to_vec();
    body.extend_from_slice(desc.as_bytes());
    body.extend_from_slice(&[0, 1, 1]);
    let mut frame = b"GEOB".to_vec();
    frame.extend_from_slice(&u32::try_from(body.len()).expect("small").to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&body);
    frame
}

/// A 24-bit stereo 44.1 kHz WAV whose `id3 ` chunk holds `frames` and 32 bytes of padding.
fn wav_with(frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = frames.concat();
    let size = u8::try_from(body.len() + 32).expect("small tag");
    let mut tag = b"ID3\x03\x00\x00\x00\x00".to_vec();
    tag.extend_from_slice(&[size >> 7, size & 0x7F]);
    tag.extend_from_slice(&body);
    tag.resize(tag.len() + 32, 0);
    let mut fmt = 1_u16.to_le_bytes().to_vec();
    fmt.extend_from_slice(&2_u16.to_le_bytes());
    fmt.extend_from_slice(&44_100_u32.to_le_bytes());
    fmt.extend_from_slice(&(44_100_u32 * 6).to_le_bytes());
    fmt.extend_from_slice(&6_u16.to_le_bytes());
    fmt.extend_from_slice(&24_u16.to_le_bytes());
    let mut riff = b"WAVE".to_vec();
    for (id, payload) in [(b"fmt ", fmt), (b"data", vec![0; 600]), (b"id3 ", tag)] {
        riff.extend_from_slice(id);
        riff.extend_from_slice(&u32::try_from(payload.len()).expect("small").to_le_bytes());
        riff.extend_from_slice(&payload);
        if payload.len() % 2 == 1 {
            riff.push(0);
        }
    }
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&u32::try_from(riff.len()).expect("small").to_le_bytes());
    file.extend_from_slice(&riff);
    file
}

fn read(dir: &Path, name: &str, frames: &[Vec<u8>]) -> ExportSource {
    let path = dir.join(name);
    std::fs::write(&path, wav_with(frames)).expect("written");
    ExportSource::read(&path)
}

#[test]
fn serato_tags_in_the_file_block_an_in_place_cut() {
    let dir = tempfile::tempdir().expect("temp dir");
    let serato = read(
        dir.path(),
        "serato.wav",
        &[geob("Serato Markers2"), geob("Serato BeatGrid")],
    );
    assert_eq!(serato.serato, SeratoPresence::Present);
    assert!(serato.has_tag);
    assert_eq!(serato.bits_per_sample, Some(24));
    // Bar 1 at 2.300 s: Prepare would cut 13,009 frames.
    let r = record(44_100, 101_430);
    let cut_s = SampleIndex(13_009).to_seconds(44_100);
    assert_eq!(
        plan_with(&r, &serato, &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::SeratoInPlaceCut { cut_s }
        }
    );
    let folder = ExportSettings {
        place: Place::Folder,
        ..prepare()
    };
    let copy = written(plan_with(&r, &serato, &folder));
    assert_eq!(copy.notices, [ExportNotice::SeratoCuesShifted { cut_s }]);
    // Library mode keeps the length, so the file is written in place.
    let library = ExportSettings::new(BatchMode::Library);
    assert!(matches!(
        plan_with(&r, &serato, &library),
        ExportOutcome::Write { .. }
    ));
}

#[test]
fn other_objects_are_not_serato_data() {
    let dir = tempfile::tempdir().expect("temp dir");
    let traktor = read(dir.path(), "traktor.wav", &[geob("Traktor4")]);
    assert_eq!(traktor.serato, SeratoPresence::Absent);
    assert!(traktor.has_tag);
    let r = record(44_100, 101_430);
    let cut = written(plan_with(&r, &traktor, &prepare()));
    assert_eq!((cut.trim_frames, cut.notices.len()), (13_009, 0));
}

#[test]
fn the_probe_answer_carries_serato_data() {
    // A FLAC row's probe found `SERATO_*` fields: the planner sees them too.
    let info = FileInfo {
        codec: Codec::Flac,
        bits_per_sample: Some(16),
        serato: true,
        serato_tags: vec![SeratoTag::Markers],
        ..FileInfo::default()
    };
    let source = ExportSource::from_info(&info);
    assert_eq!(source.serato, SeratoPresence::Present);
    let r = record(44_100, 101_430);
    assert!(matches!(
        plan_with(&r, &source, &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::SeratoInPlaceCut { .. }
        }
    ));
}

#[test]
fn tags_that_could_not_be_read_block_an_in_place_cut() {
    let info = FileInfo {
        codec: Codec::Wav,
        bits_per_sample: Some(24),
        serato_unknown: true,
        ..FileInfo::default()
    };
    let source = ExportSource::from_info(&info);
    assert_eq!(source.serato, SeratoPresence::Unknown);
    let r = record(44_100, 101_430);
    let cut_s = SampleIndex(13_009).to_seconds(44_100);
    assert_eq!(
        plan_with(&r, &source, &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::SeratoUnknownInPlaceCut { cut_s }
        }
    );
    // A copy leaves the original as it is; Library mode and a file already on a bar line move
    // nothing.
    let folder = ExportSettings {
        place: Place::Folder,
        ..prepare()
    };
    assert_eq!(written(plan_with(&r, &source, &folder)).trim_frames, 13_009);
    let library = ExportSettings::new(BatchMode::Library);
    assert!(matches!(
        plan_with(&r, &source, &library),
        ExportOutcome::Write { .. }
    ));
    let on_bar = written(plan_with(&record(48_000, 240), &source, &prepare()));
    assert_eq!(on_bar.trim_frames, 0);
}
