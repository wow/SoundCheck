//! Unit tests of `crates/sc-io/src/txn/copy_at.rs`.
use std::path::{Path, PathBuf};

use super::*;
use crate::txn::fsx::{hash_file, hex};
use crate::txn::journal::TxnKind;
use crate::txn::sidecar::{FileHash, SidecarDoc, hash_of_path};

/// What a sidecar next to `file` says it was made from.
#[derive(Default)]
struct MadeFrom {
    /// The resolved path, hashed.
    path: Option<PathBuf>,
    /// The bytes of the original, hashed.
    original: Option<PathBuf>,
}

/// A sidecar next to `file` recording `kind`, with the file as it is now as its output.
fn sidecar_of(file: &Path, kind: TxnKind, from: &MadeFrom) {
    let hashed = |p: &Path| {
        let (bytes, hash) = hash_file(p).expect("hash");
        FileHash {
            blake3: hex(&hash),
            bytes,
        }
    };
    let mut doc: SidecarDoc = serde_json::from_value(serde_json::json!({
        "schema": 2,
        "app": "SoundCheck",
        "version": "0",
        "file": "a.wav",
        "transaction": "1-1-1",
        "processed_at": "2026-11-05T00:00:00Z",
        "mode": kind,
        "original": null,
        "output": hashed(file),
        "request": {"gain_db": 0.0, "trim_frames": 0, "bits": null, "loudness": null, "tag_edits": []},
        "render": {
            "frames_in": 1, "frames_out": 1, "sample_rate_hz": 44100, "channels": 2, "bits_out": 16,
            "exact": true, "dithered": false, "samples_saturated": 0, "pcm_blake3": "00",
            "blocks": {"carried": 0, "patched": 0, "edited": 0, "replaced": 0, "dropped": 0},
            "tags_added": false, "tags_not_added": null, "stale_loudness_tags": []
        },
        "backup": null,
        "metadata": {"mtime_kept": false, "notes": []},
    }))
    .expect("a sidecar");
    doc.original = from.original.as_deref().map(hashed);
    doc.source_path_hash = from.path.as_deref().map(hash_of_path);
    std::fs::write(sidecar_path(file), serde_json::to_vec(&doc).expect("json"))
        .expect("sidecar written");
}

struct Case {
    _dir: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
}

fn case() -> Case {
    let dir = tempfile::tempdir().expect("temp");
    let base = dir.path().canonicalize().expect("resolves");
    let source = base.join("source.wav");
    let target = base.join("a.wav");
    std::fs::write(&source, b"source").expect("source");
    std::fs::write(&target, b"copy").expect("copy");
    Case {
        _dir: dir,
        source,
        target,
    }
}

/// Made from `c.source` by its path.
fn by_path(c: &Case) -> MadeFrom {
    MadeFrom {
        path: Some(c.source.clone()),
        original: None,
    }
}

#[test]
fn nothing_there() {
    let c = case();
    std::fs::remove_file(&c.target).expect("removed");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Nothing);
}

#[test]
fn a_copy_made_from_this_path_is_ours() {
    let c = case();
    sidecar_of(&c.target, TxnKind::ToFolder, &by_path(&c));
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Own);
}

#[test]
fn a_file_moved_unchanged_is_known_by_its_bytes() {
    let c = case();
    let moved = c.source.with_file_name("moved.wav");
    std::fs::copy(&c.source, &moved).expect("moved");
    let from = MadeFrom {
        path: Some(c.source.clone()),
        original: Some(c.source.clone()),
    };
    sidecar_of(&c.target, TxnKind::ToFolder, &from);
    assert_eq!(copy_at(&c.target, &moved).unwrap(), CopyAt::Own);
}

#[test]
fn a_sidecar_from_before_the_path_goes_by_the_bytes_alone() {
    let c = case();
    let from = MadeFrom {
        path: None,
        original: Some(c.source.clone()),
    };
    sidecar_of(&c.target, TxnKind::ToFolder, &from);
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Own);
    // At worst it refuses: an edited source no longer proves it.
    std::fs::write(&c.source, b"edited").expect("edited");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::OtherTrack);
}

#[test]
fn a_copy_of_an_edited_file_at_its_path_is_still_its_own() {
    let c = case();
    sidecar_of(&c.target, TxnKind::ToFolder, &by_path(&c));
    std::fs::write(&c.source, b"edited").expect("edited");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Own);
}

#[test]
fn a_copy_of_another_track_of_that_name_is_not_ours_to_replace() {
    let c = case();
    let other = c.source.with_file_name("other.wav");
    std::fs::write(&other, b"other track").expect("other");
    let from = MadeFrom {
        path: Some(other.clone()),
        original: Some(other),
    };
    sidecar_of(&c.target, TxnKind::ToFolder, &from);
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::OtherTrack);
}

#[test]
fn a_file_with_no_sidecar_is_foreign() {
    let c = case();
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn a_copy_changed_since_is_foreign() {
    let c = case();
    sidecar_of(&c.target, TxnKind::ToFolder, &by_path(&c));
    std::fs::write(&c.target, b"edit").expect("edited, same length");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn a_file_changed_in_place_is_not_a_copy() {
    let c = case();
    sidecar_of(&c.target, TxnKind::InPlace, &by_path(&c));
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn the_source_itself_is_never_its_own_copy() {
    let c = case();
    let from = MadeFrom {
        path: Some(c.source.clone()),
        original: Some(c.source.clone()),
    };
    sidecar_of(&c.source, TxnKind::ToFolder, &from);
    assert_eq!(copy_at(&c.source, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn a_link_to_a_proven_copy_is_never_taken_for_its_earlier_copy() {
    let c = case();
    let link = c.source.with_file_name("link.wav");
    std::os::unix::fs::symlink(&c.target, &link).expect("link");
    let from = MadeFrom {
        path: Some(c.target.clone()),
        original: Some(c.target.clone()),
    };
    sidecar_of(&c.target, TxnKind::ToFolder, &from);
    assert_eq!(copy_at(&c.target, &link).unwrap(), CopyAt::Foreign);
}

#[test]
fn an_unreadable_sidecar_is_foreign() {
    let c = case();
    std::fs::write(sidecar_path(&c.target), b"{").expect("written");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}
