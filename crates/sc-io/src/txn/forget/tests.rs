//! Unit tests of `forget.rs`: a pending change on an unreachable volume is forgotten with its
//! backup kept and recovery skips it; ended, unknown and running changes are refused.

use std::path::PathBuf;

use super::*;
use crate::txn::recover;

/// A journaled change's id, in the generated shape.
const ID: &str = "6ac5b4f4-100-0";

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
    let mut planned = Line::new(ID, State::Planned);
    planned.kind = Some(TxnKind::InPlace);
    planned.path = Some(target.clone());
    planned.source = Some(target.clone());
    planned.temp = Some(base.join("Volumes/Gone/.a.wav.soundcheck-tmp-t"));
    journal.append(&planned).expect("planned");
    let mut backed = Line::new(ID, State::BackedUp);
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
    assert!(!report.pending[0].to_folder, "it changed the file in place");

    let f = forget(&s.root, ID).expect("forgotten");
    assert_eq!(f.path, s.target);
    assert_eq!(f.reached, State::BackedUp);
    assert_eq!(f.backup.as_deref(), Some(s.backup.as_path()));
    // The temp file's folder cannot be reached, so it is listed; the backup exists.
    assert_eq!(f.kept.len(), 2, "{:?}", f.kept);
    assert!(f.kept.contains(&s.backup));
    assert!(s.backup.exists(), "nothing deleted");

    let e = s.journal.entry(ID).expect("read").expect("entry");
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
        forget(&s.root, "6ac5b4f4-1-9"),
        Err(Error::InvalidArgument(m)) if m.contains("no change 6ac5b4f4-1-9")
    ));
    let lock = TxnLock::try_acquire(&s.journal, ID)
        .expect("io")
        .expect("free");
    assert!(matches!(
        forget(&s.root, ID),
        Err(Error::InvalidArgument(m)) if m.contains("running")
    ));
    drop(lock);
    s.journal.append(&Line::new(ID, State::Done)).expect("done");
    assert!(matches!(
        forget(&s.root, ID),
        Err(Error::InvalidArgument(m)) if m.contains("already done")
    ));
    let e = s.journal.entry(ID).expect("read").expect("entry");
    assert_eq!(e.state, State::Done, "a refused forget writes nothing");
    let missing = s.root.join("missing");
    assert!(matches!(
        forget(&missing, ID),
        Err(Error::InvalidArgument(_))
    ));
    assert!(!missing.exists(), "no backup root is created");
}

#[test]
fn an_id_that_is_a_path_touches_nothing() {
    let s = scene();
    let base = s.root.parent().expect("base").to_path_buf();
    let victim = base.join("victim.lock");
    std::fs::write(&victim, b"not ours").expect("victim");
    let locks = s.root.join("locks");
    let before: Vec<_> = std::fs::read_dir(&locks)
        .expect("locks")
        .flatten()
        .map(|e| e.path())
        .collect();
    for id in [
        "../../victim",
        "../victim",
        "6ac5b4f4-1-0/../../victim",
        "",
        "t",
    ] {
        assert!(
            matches!(forget(&s.root, id), Err(Error::InvalidArgument(m)) if m.contains("not a change id")),
            "{id:?}"
        );
    }
    assert_eq!(std::fs::read(&victim).expect("still there"), b"not ours");
    let after: Vec<_> = std::fs::read_dir(&locks)
        .expect("locks")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(before, after, "the lock folder is untouched");
    assert!(TxnLock::try_acquire(&s.journal, "../../victim").is_err());
    assert!(victim.exists());
}
/// A journal in `root` holding a copy into `<dir>/a.wav` that reached `reached`.
fn copy_to(root: &std::path::Path, dir: &std::path::Path, reached: State) -> (Journal, PathBuf) {
    let journal = Journal::open(root).expect("journal");
    let target = dir.join("a.wav");
    let mut planned = Line::new(ID, State::Planned);
    planned.kind = Some(TxnKind::ToFolder);
    planned.path = Some(target.clone());
    planned.source = Some(root.join("a.wav"));
    planned.temp = Some(dir.join(".a.wav.soundcheck-tmp-t"));
    planned.output_bytes = Some(4096);
    journal.append(&planned).expect("planned");
    for state in [State::TempWritten, State::Verified, State::Renamed] {
        if state <= reached {
            journal.append(&Line::new(ID, state)).expect("step");
        }
    }
    (journal, target)
}

#[test]
fn a_copy_to_an_unmounted_stick_lists_the_placeholder_it_may_have_left() {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let root = base.join("backups");
    // Whether the empty file is on the stick cannot be known while it is away, so it is listed.
    let gone = base.join("Volumes/STICK/Crate");
    let (_journal, target) = copy_to(&root, &gone, State::Verified);
    let report = recover(&root).expect("recovery runs");
    let pending = report.pending.first().expect("unreachable, so pending");
    assert!(pending.to_folder, "recovery says it was a copy");
    let f = forget(&root, ID).expect("forgotten");
    assert!(f.kept.contains(&target), "{:?}", f.kept);
    assert_eq!(f.backup, None, "a copy makes no backup");
}

#[test]
fn an_empty_placeholder_is_listed_and_kept_but_a_real_file_is_not() {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let folder = base.join("Crate");
    std::fs::create_dir_all(&folder).expect("folder");

    let root = base.join("backups");
    let (_journal, target) = copy_to(&root, &folder, State::Verified);
    std::fs::write(&target, b"").expect("placeholder");
    let f = forget(&root, ID).expect("forgotten");
    assert!(f.kept.contains(&target), "{:?}", f.kept);
    assert!(target.exists(), "nothing deleted");

    // A file of that name with something in it is not the change's placeholder.
    let root = base.join("backups-2");
    let (_journal, target) = copy_to(&root, &folder, State::Verified);
    std::fs::write(&target, b"someone else's").expect("other file");
    let f = forget(&root, ID).expect("forgotten");
    assert!(!f.kept.contains(&target), "{:?}", f.kept);

    // Past the rename, the file there is the copy itself, not a placeholder.
    let root = base.join("backups-3");
    let (_journal, target) = copy_to(&root, &folder, State::Renamed);
    std::fs::write(&target, b"").expect("empty");
    let f = forget(&root, ID).expect("forgotten");
    assert!(!f.kept.contains(&target), "{:?}", f.kept);
}
