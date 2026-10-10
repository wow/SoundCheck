//! Unit tests of `meta.rs` (macOS): the resource fork carried as readers see it.
#![cfg(target_os = "macos")]

use super::*;

/// The resource fork of `p`, read through its named fork.
fn fork_of(p: &Path) -> Vec<u8> {
    std::fs::read(p.join("..namedfork/rsrc")).expect("resource fork")
}

/// Writing a fork overwrites only its start (`setxattr(2)` at position 0), so a longer fork on
/// the target must be removed first, or its tail stays.
#[test]
fn a_longer_resource_fork_on_the_target_is_replaced_not_overwritten() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (source, target) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
    std::fs::write(&source, b"source").expect("source");
    std::fs::write(&target, b"target").expect("target");
    let short: Vec<u8> = (0..3_000_u32).map(|i| (i % 251) as u8).collect();
    let long: Vec<u8> = (0..83_000_u32).map(|i| (i % 241) as u8).collect();
    // Set through the attribute, not written through a descriptor: a process spawned by another
    // test at the moment such a descriptor closes would keep the fork pending (unlisted, and
    // `removexattr(2)` refusing it with EBUSY).
    xattr::set(&source, RESOURCE_FORK, &short).expect("short fork");
    xattr::set(&target, RESOURCE_FORK, &long).expect("long fork");
    let meta = snapshot(&source).expect("snapshot");
    let notes = restore(&target, &meta, true).expect("restored");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(fork_of(&target), short);
}

/// An empty named fork (HFS+ shows one on every file) is no fork: nothing to carry.
#[test]
fn a_file_without_a_resource_fork_carries_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("a.wav");
    std::fs::write(&path, b"no fork").expect("file");
    let meta = snapshot(&path).expect("snapshot");
    assert!(
        !meta.xattrs.iter().any(|(n, _)| n == RESOURCE_FORK),
        "{:?}",
        meta.xattrs
    );
}

/// The system stamps a new file with its own `com.apple.provenance` and ignores a write of
/// another value, so it is not carried and no note says it differs; other attributes still are.
#[test]
fn provenance_is_left_to_the_system_without_a_note() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (source, target) = (dir.path().join("a.flac"), dir.path().join("b.flac"));
    std::fs::write(&source, b"source").expect("source");
    std::fs::write(&target, b"target").expect("target");
    xattr::set(&source, "com.example.kept", b"kept").expect("an ordinary attribute");
    let mut meta = snapshot(&source).expect("snapshot");
    let provenance = (
        OsString::from("com.apple.provenance"),
        vec![1, 0, 0, 0x5f, 0x3e, 0x88, 0x88, 0xda, 0x67, 0x79, 0xe0],
    );
    let at = meta
        .xattrs
        .binary_search_by(|(n, _)| n.cmp(&provenance.0))
        .unwrap_or_else(|i| i);
    if meta.xattrs.get(at).is_some_and(|(n, _)| *n == provenance.0) {
        meta.xattrs[at] = provenance;
    } else {
        meta.xattrs.insert(at, provenance);
    }
    let notes = restore(&target, &meta, true).expect("restored");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        xattr::get(&target, "com.example.kept").expect("readable"),
        Some(b"kept".to_vec())
    );
}
