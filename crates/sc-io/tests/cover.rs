//! The embedded cover for the grid view's header: the front cover is found in a FLAC's picture
//! block, recognised by its bytes, and left out when it is too large or not an image.

use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Tag, TagExt};
use sc_io::tags::{MAX_COVER_BYTES, cover};

fn fixture_copy(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/sine-2s.flac");
    let dst = dir.join("with-cover.flac");
    std::fs::copy(src, &dst).unwrap();
    dst
}

/// Writes `pictures` into the file's tag (the test's own tagging; SoundCheck never writes with
/// lofty).
fn tag_with(path: &Path, pictures: Vec<Picture>) {
    let file = lofty::read_from_path(path).unwrap();
    let mut tag = file
        .primary_tag()
        .cloned()
        .unwrap_or_else(|| Tag::new(file.primary_tag_type()));
    for p in pictures {
        tag.push_picture(p);
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

fn picture(kind: PictureType, bytes: Vec<u8>) -> Picture {
    Picture::unchecked(bytes)
        .pic_type(kind)
        .mime_type(MimeType::Png)
        .build()
}

fn png(extra: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    b.resize(8 + extra, 7);
    b
}

#[test]
fn the_front_cover_is_found_and_recognised() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture_copy(dir.path());
    assert_eq!(cover(&path), None, "no picture yet");
    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    tag_with(
        &path,
        vec![
            picture(PictureType::Artist, png(100)),
            picture(PictureType::CoverFront, jpeg.clone()),
        ],
    );
    let found = cover(&path).unwrap();
    assert_eq!(
        found.mime, "image/jpeg",
        "sniffed from the bytes, not the declared type"
    );
    assert_eq!(found.bytes, jpeg);
}

#[test]
fn oversized_or_unknown_pictures_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture_copy(dir.path());
    tag_with(
        &path,
        vec![picture(PictureType::CoverFront, png(MAX_COVER_BYTES))],
    );
    assert_eq!(cover(&path), None, "larger than the cap");
    let path = fixture_copy(dir.path());
    tag_with(
        &path,
        vec![picture(PictureType::CoverFront, b"not an image".to_vec())],
    );
    assert_eq!(cover(&path), None);
    assert_eq!(cover(Path::new("/nonexistent.flac")), None);
}
