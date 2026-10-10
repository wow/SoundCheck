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
    std::fs::write(source.join("..namedfork/rsrc"), &short).expect("short fork");
    std::fs::write(target.join("..namedfork/rsrc"), &long).expect("long fork");
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
