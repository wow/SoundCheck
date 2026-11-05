//! Unit tests of `crates/sc-io/src/txn/copy_at.rs`.
use std::path::Path;

use super::*;
use crate::txn::fsx::{hash_file, hex};
use crate::txn::journal::TxnKind;
use crate::txn::sidecar::{FileHash, SidecarDoc};

/// A sidecar next to `file` recording `kind`, with `bytes` and `blake3` as its output.
fn sidecar_of(file: &Path, kind: TxnKind, bytes: u64, blake3: String) {
    let doc: SidecarDoc = serde_json::from_value(serde_json::json!({
        "schema": 2,
        "app": "SoundCheck",
        "version": "0",
        "file": "a.wav",
        "transaction": "1-1-1",
        "processed_at": "2026-11-05T00:00:00Z",
        "mode": kind,
        "original": null,
        "output": FileHash { blake3, bytes },
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
    std::fs::write(sidecar_path(file), serde_json::to_vec(&doc).expect("json"))
        .expect("sidecar written");
}

fn hashed(file: &Path) -> (u64, String) {
    let (bytes, hash) = hash_file(file).expect("hash");
    (bytes, hex(&hash))
}

struct Case {
    _dir: tempfile::TempDir,
    source: std::path::PathBuf,
    target: std::path::PathBuf,
}

fn case() -> Case {
    let dir = tempfile::tempdir().expect("temp");
    let source = dir.path().join("source.wav");
    let target = dir.path().join("a.wav");
    std::fs::write(&source, b"source").expect("source");
    std::fs::write(&target, b"copy").expect("copy");
    Case {
        _dir: dir,
        source,
        target,
    }
}

#[test]
fn nothing_there() {
    let c = case();
    std::fs::remove_file(&c.target).expect("removed");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Nothing);
}

#[test]
fn a_copy_its_sidecar_proves_is_ours() {
    let c = case();
    let (bytes, hash) = hashed(&c.target);
    sidecar_of(&c.target, TxnKind::ToFolder, bytes, hash);
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Own);
}

#[test]
fn a_file_with_no_sidecar_is_foreign() {
    let c = case();
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn a_copy_changed_since_is_foreign() {
    let c = case();
    let (bytes, hash) = hashed(&c.target);
    sidecar_of(&c.target, TxnKind::ToFolder, bytes, hash);
    std::fs::write(&c.target, b"edit").expect("edited, same length");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn a_file_changed_in_place_is_not_a_copy() {
    let c = case();
    let (bytes, hash) = hashed(&c.target);
    sidecar_of(&c.target, TxnKind::InPlace, bytes, hash);
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn the_source_itself_is_never_its_own_copy() {
    let c = case();
    let (bytes, hash) = hashed(&c.source);
    sidecar_of(&c.source, TxnKind::ToFolder, bytes, hash);
    assert_eq!(copy_at(&c.source, &c.source).unwrap(), CopyAt::Foreign);
}

#[test]
fn an_unreadable_sidecar_is_foreign() {
    let c = case();
    std::fs::write(sidecar_path(&c.target), b"{").expect("written");
    assert_eq!(copy_at(&c.target, &c.source).unwrap(), CopyAt::Foreign);
}
