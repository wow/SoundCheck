//! Unit tests of `forget.rs`: a pending change on an unreachable volume is forgotten with its
//! backup kept and recovery skips it; ended, unknown and running changes are refused.

use std::path::PathBuf;

use super::*;
use crate::txn::recover;

/// A journal holding an in-place change of `<gone>/a.wav` (a folder that does not exist, as on
/// an unmounted volume) that reached `backed_up`, with its backup present.
struct Scene {
    _dir: tempfile::TempDir,
    root: PathBuf,
    target: PathBuf,
    backup: PathBuf,
    journal: Journal,
}

fn scene() -> Scene {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let root = base.join("backups");
    let target = base.join("Volumes/Gone/a.wav");
    let backup = root.join("2026-10-07/Gone/a.wav");
    let journal = Journal::open(&root).expect("journal");
    let mut planned = Line::new("t", State::Planned);
    planned.kind = Some(TxnKind::InPlace);
    planned.path = Some(target.clone());
    planned.source = Some(target.clone());
    planned.temp = Some(base.join("Volumes/Gone/.a.wav.soundcheck-tmp-t"));
    journal.append(&planned).expect("planned");
    let mut backed = Line::new("t", State::BackedUp);
    backed.backup = Some(backup.clone());
    journal.append(&backed).expect("backed up");
    std::fs::create_dir_all(backup.parent().expect("parent")).expect("backup folder");
    std::fs::write(&backup, b"original").expect("backup");
    Scene {
        _dir: dir,
        root,
        target,
        backup,
        journal,
    }
}

#[test]
fn a_pending_change_is_forgotten_and_recovery_skips_it() {
    let s = scene();
    let report = recover(&s.root).expect("recovery runs");
    assert_eq!(report.pending.len(), 1, "unreachable, so pending");

    let f = forget(&s.root, "t").expect("forgotten");
    assert_eq!(f.path, s.target);
    assert_eq!(f.reached, State::BackedUp);
    assert_eq!(f.backup.as_deref(), Some(s.backup.as_path()));
    // The temp file's folder cannot be reached, so it is listed; the backup exists.
    assert_eq!(f.kept.len(), 2, "{:?}", f.kept);
    assert!(f.kept.contains(&s.backup));
    assert!(s.backup.exists(), "nothing deleted");

    let e = s.journal.entry("t").expect("read").expect("entry");
    assert_eq!(e.state, State::Forgotten);
    assert_eq!(e.reached, State::BackedUp);
    assert!(!e.completed());
    let report = recover(&s.root).expect("recovery runs");
    assert!(report.pending.is_empty() && report.recovered.is_empty());
}

#[test]
fn ended_unknown_and_running_changes_are_refused() {
    let s = scene();
    assert!(matches!(
        forget(&s.root, "nope"),
        Err(Error::InvalidArgument(m)) if m.contains("no change nope")
    ));
    let lock = TxnLock::try_acquire(&s.journal, "t")
        .expect("io")
        .expect("free");
    assert!(matches!(
        forget(&s.root, "t"),
        Err(Error::InvalidArgument(m)) if m.contains("running")
    ));
    drop(lock);
    s.journal
        .append(&Line::new("t", State::Done))
        .expect("done");
    assert!(matches!(
        forget(&s.root, "t"),
        Err(Error::InvalidArgument(m)) if m.contains("already done")
    ));
    let e = s.journal.entry("t").expect("read").expect("entry");
    assert_eq!(e.state, State::Done, "a refused forget writes nothing");
    let missing = s.root.join("missing");
    assert!(matches!(
        forget(&missing, "t"),
        Err(Error::InvalidArgument(_))
    ));
    assert!(!missing.exists(), "no backup root is created");
}
