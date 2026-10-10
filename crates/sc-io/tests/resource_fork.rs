//! macOS: the resource fork of a file changed in place reaches the output, the backup and the
//! undo whole: a large one (more than one 64 KiB read), one still open for writing (not yet
//! listed as an extended attribute), and one extended through a descriptor still open (listed,
//! but `getxattr(2)` still returns the old bytes). The stress run is `tests/fork_stress.rs`.
#![cfg(target_os = "macos")]

mod common;

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use common::{Library, blake3_of, wav};
use sc_core::RenderRequest;
use sc_io::txn::{self, TxnOptions};

static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

const RESOURCE_FORK: &str = "com.apple.ResourceFork";

fn gain(gain_db: f64) -> RenderRequest {
    RenderRequest {
        gain_db,
        ..RenderRequest::default()
    }
}

fn opts(lib: &Library) -> TxnOptions {
    TxnOptions::new(&lib.backups)
}

/// The resource fork of `p`, read through its named fork.
fn read_fork(p: &Path) -> Vec<u8> {
    std::fs::read(p.join("..namedfork/rsrc"))
        .unwrap_or_else(|e| panic!("resource fork of {}: {e}", p.display()))
}

/// `len` bytes of a resource fork, a pattern of period `period`.
fn test_fork(len: u32, period: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % u32::from(period)).expect("below the period, a byte"))
        .collect()
}

/// Whether the resource fork of `p` is listed as an extended attribute.
fn listed(p: &Path) -> bool {
    xattr::list(p)
        .expect("extended attributes")
        .any(|n| n.as_os_str() == RESOURCE_FORK)
}

/// The size `getxattr(2)` gives the resource fork of `p`.
fn attribute_size(p: &Path) -> usize {
    rustix::fs::getxattr(p, RESOURCE_FORK, &mut [0_u8; 0][..]).expect("resource fork attribute")
}

/// A 70,000-byte resource fork (reads of it are cut to the buffer unless sized first).
#[test]
fn a_large_resource_fork_survives_in_the_output_the_backup_and_the_undo() {
    let lib = Library::new();
    let path = lib.add("fork.wav", &wav(3000, 31));
    let fork = test_fork(70_000, 253);
    std::fs::write(path.join("..namedfork/rsrc"), &fork).expect("resource fork written");
    assert_eq!(read_fork(&path), fork, "before apply");
    let original = blake3_of(&path);
    let report =
        txn::apply_in_place(&path, &gain(-2.0), &opts(&lib), &NOT_CANCELLED).expect("applied");
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    assert_eq!(read_fork(&path).len(), fork.len());
    assert_eq!(read_fork(&path), fork, "output");
    let backup = report.backup.expect("backup");
    assert_eq!(read_fork(&backup), fork, "backup");
    let undone = txn::undo(&path, &lib.backups).expect("undone");
    assert!(undone.notes.is_empty(), "{:?}", undone.notes);
    assert_eq!(blake3_of(&path), original);
    assert_eq!(read_fork(&path), fork, "after undo");
}

/// While a descriptor that wrote the resource fork through its named fork is still open
/// (another app's, or a duplicate a child process holds for the instant it is being spawned,
/// which is how the fork written by the test above could still be pending when a test running
/// in parallel started a process), the volume neither lists nor returns the fork as an
/// extended attribute, but the system copy that makes the backup copies it. The output must
/// carry it too, not lose it without a note.
#[test]
fn regression_a_resource_fork_still_open_for_writing_reaches_the_output() {
    let lib = Library::new();
    let path = lib.add("pending.wav", &wav(3000, 35));
    let fork = test_fork(70_000, 253);
    let mut writer = std::fs::File::create(path.join("..namedfork/rsrc")).expect("fork opened");
    writer.write_all(&fork).expect("resource fork written");
    assert!(
        !listed(&path),
        "precondition: a fork open for writing is not listed as an extended attribute"
    );
    let report =
        txn::apply_in_place(&path, &gain(-2.0), &opts(&lib), &NOT_CANCELLED).expect("applied");
    drop(writer);
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    assert_eq!(read_fork(&path), fork, "output");
    assert_eq!(read_fork(&report.backup.expect("backup")), fork, "backup");
    let undone = txn::undo(&path, &lib.backups).expect("undone");
    assert!(undone.notes.is_empty(), "{:?}", undone.notes);
    assert_eq!(read_fork(&path), fork, "after undo");
}

/// A committed 3,000-byte fork extended by 80,000 bytes through a descriptor still open: the
/// fork is listed, but its attribute still reads the 3,000 committed bytes while the named
/// fork and the system copy have all 83,000. The output must get all of them, not the stale
/// start (with a note blaming the backup, the copy that is whole).
#[test]
fn regression_a_resource_fork_extended_by_an_open_writer_reaches_the_output_whole() {
    let lib = Library::new();
    let path = lib.add("stale.wav", &wav(3000, 36));
    let start = test_fork(3_000, 251);
    std::fs::write(path.join("..namedfork/rsrc"), &start).expect("resource fork written");
    assert!(listed(&path), "precondition: the closed fork is listed");
    let more = test_fork(80_000, 241);
    let mut writer = std::fs::OpenOptions::new()
        .append(true)
        .open(path.join("..namedfork/rsrc"))
        .expect("fork opened to append");
    writer.write_all(&more).expect("resource fork extended");
    let whole = [start, more].concat();
    assert_eq!(
        read_fork(&path),
        whole,
        "the named fork reads the new bytes"
    );
    assert_eq!(
        (listed(&path), attribute_size(&path)),
        (true, 3_000),
        "precondition: while the writer is open, the attribute reads the committed bytes"
    );
    let original = blake3_of(&path);
    let report =
        txn::apply_in_place(&path, &gain(-2.0), &opts(&lib), &NOT_CANCELLED).expect("applied");
    drop(writer);
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    assert_eq!(read_fork(&path), whole, "output");
    assert_eq!(read_fork(&report.backup.expect("backup")), whole, "backup");
    let undone = txn::undo(&path, &lib.backups).expect("undone");
    assert!(undone.notes.is_empty(), "{:?}", undone.notes);
    assert_eq!(blake3_of(&path), original);
    assert_eq!(read_fork(&path), whole, "after undo");
}
