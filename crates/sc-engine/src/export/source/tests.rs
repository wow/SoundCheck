//! Unit tests of `crates/sc-engine/src/export/source.rs`.
use super::*;

/// A RIFF/WAVE file of `chunks` (id, payload), pad bytes added.
fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = b"WAVE".to_vec();
    for (id, payload) in chunks {
        body.extend_from_slice(*id);
        body.extend_from_slice(&u32::try_from(payload.len()).expect("small").to_le_bytes());
        body.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&u32::try_from(body.len()).expect("small").to_le_bytes());
    file.extend_from_slice(&body);
    file
}

/// A `fmt ` payload: format tag, 2 channels, 44.1 kHz, `bits` per sample.
fn fmt(tag: u16, bits: u16) -> Vec<u8> {
    let align = 2 * bits / 8;
    let mut f = Vec::new();
    f.extend_from_slice(&tag.to_le_bytes());
    f.extend_from_slice(&2_u16.to_le_bytes());
    f.extend_from_slice(&44_100_u32.to_le_bytes());
    f.extend_from_slice(&(44_100 * u32::from(align)).to_le_bytes());
    f.extend_from_slice(&align.to_le_bytes());
    f.extend_from_slice(&bits.to_le_bytes());
    f
}

/// An empty ID3v2.3 tag with 16 bytes of padding.
fn id3() -> Vec<u8> {
    let mut tag = b"ID3\x03\x00\x00\x00\x00\x00\x10".to_vec();
    tag.resize(tag.len() + 16, 0);
    tag
}

fn read(dir: &Path, name: &str, bytes: &[u8]) -> ExportSource {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("written");
    ExportSource::read(&path)
}

#[test]
fn reads_depth_float_and_the_tag_from_the_chunks() {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = (b"data", vec![0_u8; 4 * 6 * 10]);
    let tagged = read(
        dir.path(),
        "tagged.wav",
        &riff(&[(b"fmt ", fmt(1, 24)), data.clone(), (b"id3 ", id3())]),
    );
    assert_eq!(tagged.codec, Codec::Wav);
    assert_eq!((tagged.bits_per_sample, tagged.float), (Some(24), false));
    assert!(tagged.has_tag && !tagged.serato && tagged.blake3.is_none());

    let bare = read(
        dir.path(),
        "bare.wav",
        &riff(&[(b"fmt ", fmt(1, 24)), data.clone()]),
    );
    assert!(!bare.has_tag);

    // Two tags: the writer edits neither, so there is no tag to write into.
    let two = read(
        dir.path(),
        "two.wav",
        &riff(&[
            (b"fmt ", fmt(1, 24)),
            data.clone(),
            (b"id3 ", id3()),
            (b"ID3 ", id3()),
        ]),
    );
    assert!(!two.has_tag);

    let float = read(
        dir.path(),
        "float.wav",
        &riff(&[(b"fmt ", fmt(3, 32)), data]),
    );
    assert_eq!((float.bits_per_sample, float.float), (Some(32), true));
}

#[test]
fn an_unreadable_file_keeps_the_name_s_codec() {
    let dir = tempfile::tempdir().expect("temp dir");
    let junk = read(dir.path(), "junk.aiff", b"not audio");
    assert_eq!(junk.codec, Codec::Aiff);
    assert!(!junk.has_tag && !junk.float);
}
