//! `probe` on the committed fixtures and on generated WAVs: codec, spec, bitrate, the DJ-safe
//! reason and tags, without decoding.

use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::tag::{Tag, TagType};
use sc_core::ipc::DjUnsafe;
use sc_core::plan::Codec;
use sc_io::probe;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

fn silent_wav(dir: &Path, name: &str, rate: u32, bits: u16, float: bool) -> PathBuf {
    let path = dir.join(name);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: bits,
        sample_format: if float {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create");
    for _ in 0..rate {
        if float {
            w.write_sample(0.0_f32).expect("write");
            w.write_sample(0.0_f32).expect("write");
        } else {
            w.write_sample(0_i32).expect("write");
            w.write_sample(0_i32).expect("write");
        }
    }
    w.finalize().expect("finalize");
    path
}

#[test]
fn mp3_fixture_reports_its_bitrate() {
    let info = probe(&fixture("lame-2s.mp3"));
    assert_eq!(info.codec, Codec::Mp3);
    assert_eq!(info.sample_rate, Some(44_100));
    assert_eq!(info.channels, Some(2));
    assert_eq!(info.bits_per_sample, None);
    assert_eq!(info.bitrate_kbps, Some(128));
    assert!(
        (info.duration.unwrap().0 - 2.0).abs() < 0.1,
        "{:?}",
        info.duration
    );
    assert_eq!(info.dj_unsafe, None);
}

#[test]
fn flac_fixture_is_dj_safe() {
    let info = probe(&fixture("sine-2s.flac"));
    assert_eq!(info.codec, Codec::Flac);
    assert_eq!((info.sample_rate, info.channels), (Some(44_100), Some(2)));
    assert_eq!(info.bits_per_sample, Some(16));
    assert_eq!(info.bitrate_kbps, None);
    assert_eq!(info.dj_unsafe, None);
}

#[test]
fn mono_24_bit_wav_is_flagged_for_its_channels() {
    let info = probe(&fixture("sine-1s-mono-24.wav"));
    assert_eq!(info.codec, Codec::Wav);
    assert_eq!(info.bits_per_sample, Some(24));
    assert!(!info.float);
    assert_eq!(info.dj_unsafe, Some(DjUnsafe::Channels));
}

#[test]
fn float_and_high_rate_wavs_are_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let info = probe(&silent_wav(dir.path(), "f.wav", 44_100, 32, true));
    assert!(info.float);
    assert_eq!(info.dj_unsafe, Some(DjUnsafe::Float));
    let info = probe(&silent_wav(dir.path(), "hr.wav", 96_000, 24, false));
    assert_eq!(info.dj_unsafe, Some(DjUnsafe::SampleRate));
    let info = probe(&silent_wav(dir.path(), "i32.wav", 48_000, 32, false));
    assert!(!info.float);
    assert_eq!(info.dj_unsafe, Some(DjUnsafe::BitDepth));
}

#[test]
fn title_artist_and_album_come_from_the_tags() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tagged.flac");
    std::fs::copy(fixture("sine-2s.flac"), &path).unwrap();
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.set_title("Şımarık".into());
    tag.set_artist("Tarkan".into());
    tag.set_album(" Ölürüm Sana ".into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    let info = probe(&path);
    assert_eq!(info.title.as_deref(), Some("Şımarık"));
    assert_eq!(info.artist.as_deref(), Some("Tarkan"));
    assert_eq!(info.album.as_deref(), Some("Ölürüm Sana"), "trimmed");
}

#[test]
fn an_unreadable_file_keeps_the_codec_its_name_suggests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.aiff");
    std::fs::write(&path, b"not audio").unwrap();
    let info = probe(&path);
    assert_eq!(info.codec, Codec::Aiff);
    assert_eq!(info.sample_rate, None);
    assert_eq!(info.dj_unsafe, None);
}
