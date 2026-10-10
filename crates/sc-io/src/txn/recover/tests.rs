//! Unit tests of `recover.rs`: the decision table, a crash between the rename and its journal
//! line (both ways), and a running transaction left alone.

use std::path::PathBuf;

use super::*;
use crate::txn::fsx::hex;

#[test]
fn the_decision_follows_the_furthest_state() {
    use Action::{CheckTarget, Complete, RollBack};
    let table = [
        (TxnKind::InPlace, State::Planned, true, RollBack),
        (TxnKind::InPlace, State::Planned, false, RollBack),
        (TxnKind::InPlace, State::TempWritten, true, RollBack),
        (TxnKind::InPlace, State::Verified, true, RollBack),
        (TxnKind::InPlace, State::Verified, false, RollBack),
        (TxnKind::InPlace, State::BackedUp, true, RollBack),
        (TxnKind::InPlace, State::BackedUp, false, CheckTarget),
        (TxnKind::InPlace, State::Renamed, false, Complete),
        (TxnKind::InPlace, State::MetadataDone, false, Complete),
        (TxnKind::ToFolder, State::Verified, true, RollBack),
        (TxnKind::ToFolder, State::Verified, false, CheckTarget),
        (TxnKind::Undo, State::TempWritten, true, RollBack),
        (TxnKind::Undo, State::Verified, false, CheckTarget),
        (TxnKind::Undo, State::Renamed, false, Complete),
    ];
    for (kind, reached, temp, want) in table {
        assert_eq!(
            decide(kind, reached, temp),
            want,
            "{kind:?} {reached:?} {temp}"
        );
    }
}

/// A journal holding an in-place transaction of `music/a.wav` that reached `backed_up`, with
/// the target, temp and backup files as given.
struct Scene {
    _dir: tempfile::TempDir,
    root: PathBuf,
    target: PathBuf,
    temp: PathBuf,
    backup: PathBuf,
    journal: Journal,
}

const ORIGINAL: &[u8] = b"original bytes";
const OUTPUT: &[u8] = b"rendered output bytes";

fn scene() -> Scene {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let (music, root) = (base.join("music"), base.join("backups"));
    std::fs::create_dir_all(&music).expect("music");
    let target = music.join("a.wav");
    let temp = music.join(".a.wav.soundcheck-tmp-t");
    let backup = root.join("2026-10-07/Disk/a.wav");
    let journal = Journal::open(&root).expect("journal");
    let mut planned = Line::new("t", State::Planned);
    planned.kind = Some(TxnKind::InPlace);
    planned.path = Some(target.clone());
    planned.source = Some(target.clone());
    planned.temp = Some(temp.clone());
    journal.append(&planned).expect("planned");
    let mut verified = Line::new("t", State::Verified);
    verified.output_blake3 = Some(hex(blake3::hash(OUTPUT).as_bytes()));
    journal.append(&verified).expect("verified");
    let mut backed = Line::new("t", State::BackedUp);
    backed.backup = Some(backup.clone());
    backed.original_blake3 = Some(hex(blake3::hash(ORIGINAL).as_bytes()));
    journal.append(&backed).expect("backed up");
    std::fs::create_dir_all(backup.parent().expect("parent")).expect("backup folder");
    std::fs::write(&backup, ORIGINAL).expect("backup");
    Scene {
        _dir: dir,
        root,
        target,
        temp,
        backup,
        journal,
    }
}

#[test]
fn a_rename_the_journal_missed_is_completed() {
    let s = scene();
    // Crashed after the rename, before its line: the temp is gone, the target is the output.
    std::fs::write(&s.target, OUTPUT).expect("output in place");
    let r = recover(&s.root).expect("recovered").recovered;
    assert_eq!(r.len(), 1);
    assert_eq!(
        (r[0].reached, r[0].outcome),
        (State::BackedUp, Outcome::Completed)
    );
    assert_eq!(std::fs::read(&s.target).expect("target"), OUTPUT);
    assert!(s.backup.exists(), "the backup stays");
    let e = s.journal.entry("t").expect("read").expect("entry");
    assert!(e.completed());
}

#[test]
fn an_interrupted_change_is_rolled_back_and_its_backup_removed() {
    let s = scene();
    std::fs::write(&s.target, ORIGINAL).expect("original in place");
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    let r = recover(&s.root).expect("recovered").recovered;
    assert_eq!(r[0].outcome, Outcome::RolledBack);
    assert!(r[0].notes.is_empty(), "{:?}", r[0].notes);
    assert!(!s.temp.exists() && !s.backup.exists());
    assert_eq!(std::fs::read(&s.target).expect("target"), ORIGINAL);
    assert!(
        recover(&s.root).expect("again").recovered.is_empty(),
        "idempotent"
    );
}

#[test]
fn a_backup_is_kept_when_the_file_no_longer_matches_it() {
    let s = scene();
    std::fs::write(&s.target, b"edited meanwhile").expect("target");
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    let r = recover(&s.root).expect("recovered").recovered;
    assert_eq!(r[0].outcome, Outcome::RolledBack);
    assert_eq!(r[0].notes.len(), 1, "{:?}", r[0].notes);
    assert!(s.backup.exists());
}

#[test]
fn a_running_transaction_is_left_alone() {
    let s = scene();
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    let lock = TxnLock::try_acquire(&s.journal, "t")
        .expect("io")
        .expect("free");
    assert!(
        recover(&s.root).expect("read").recovered.is_empty(),
        "skipped"
    );
    assert!(s.temp.exists());
    drop(lock);
    assert_eq!(recover(&s.root).expect("now").recovered.len(), 1);
}

#[test]
fn no_backup_root_means_nothing_to_recover() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(
        recover(&dir.path().join("none"))
            .expect("ok")
            .recovered
            .is_empty(),
        "nothing"
    );
    assert!(
        !dir.path().join("none").exists(),
        "recovery creates nothing"
    );
}

#[test]
fn a_transaction_on_an_unmounted_volume_stays_pending() {
    let s = scene();
    let music = s.target.parent().expect("folder").to_path_buf();
    // The volume is gone: its folder does not exist.
    std::fs::remove_dir_all(&music).expect("unmounted");
    let r = recover(&s.root).expect("read");
    assert!(r.recovered.is_empty(), "{r:?}");
    assert_eq!(r.pending.len(), 1);
    assert!(r.pending[0].reason.contains("not reachable"), "{r:?}");
    assert!(s.backup.exists(), "nothing touched");
    let e = s.journal.entry("t").expect("read").expect("entry");
    assert_eq!(e.state, State::BackedUp, "no rolled-back record");
    std::fs::create_dir_all(&music).expect("mounted again");
    std::fs::write(&s.target, ORIGINAL).expect("original");
    assert_eq!(recover(&s.root).expect("now").recovered.len(), 1);
}

#[test]
fn a_cleanup_that_fails_stays_pending_and_is_retried() {
    use std::os::unix::fs::PermissionsExt;
    let s = scene();
    std::fs::write(&s.target, ORIGINAL).expect("original");
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    let music = s.target.parent().expect("folder").to_path_buf();
    let mode = |m| std::fs::set_permissions(&music, std::fs::Permissions::from_mode(m));
    mode(0o555).expect("read-only folder");
    let r = recover(&s.root).expect("read");
    mode(0o755).expect("writable again");
    assert!(r.recovered.is_empty(), "{r:?}");
    assert_eq!(r.pending.len(), 1);
    let e = s.journal.entry("t").expect("read").expect("entry");
    assert!(!e.state.is_final());
    assert!(
        e.notes.iter().any(|n| n.contains("cleanup failed")),
        "{:?}",
        e.notes
    );
    let r = recover(&s.root).expect("retry");
    assert_eq!(r.recovered.len(), 1);
    assert!(!s.temp.exists() && !s.backup.exists());
}

#[test]
fn one_failing_transaction_does_not_stop_the_others() {
    use std::os::unix::fs::PermissionsExt;
    let s = scene();
    std::fs::write(&s.target, ORIGINAL).expect("original");
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    // A second transaction in another folder, interrupted after planning.
    let other_dir = s.root.parent().expect("base").join("other");
    std::fs::create_dir(&other_dir).expect("folder");
    let mut planned = Line::new("u", State::Planned);
    planned.kind = Some(TxnKind::ToFolder);
    planned.path = Some(other_dir.join("b.wav"));
    planned.source = Some(s.target.clone());
    planned.temp = Some(other_dir.join(".b.wav.soundcheck-tmp-u"));
    s.journal.append(&planned).expect("planned");
    let music = s.target.parent().expect("folder").to_path_buf();
    std::fs::set_permissions(&music, std::fs::Permissions::from_mode(0o555)).expect("ro");
    let r = recover(&s.root).expect("read");
    std::fs::set_permissions(&music, std::fs::Permissions::from_mode(0o755)).expect("rw");
    assert_eq!(r.pending.len(), 1, "{r:?}");
    assert_eq!(r.pending[0].txn, "t");
    assert_eq!(r.recovered.len(), 1);
    assert_eq!(r.recovered[0].txn, "u");
}

#[test]
fn a_file_at_the_backup_name_that_is_not_the_original_is_kept() {
    let s = scene();
    std::fs::write(&s.target, ORIGINAL).expect("original");
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    // The journal names a backup that was never made by this change; another file sits there.
    let mut line = Line::new("t", State::BackedUp);
    let stranger = s.backup.with_file_name("a (2).wav");
    line.backup_target = Some(stranger.clone());
    s.journal.append(&line).expect("named");
    std::fs::write(&stranger, b"someone else's file").expect("stranger");
    let r = recover(&s.root).expect("recovered").recovered;
    assert_eq!(r[0].outcome, Outcome::RolledBack);
    assert!(stranger.exists(), "kept");
    assert!(
        r[0].notes.iter().any(|n| n.contains("a (2).wav")),
        "{:?}",
        r[0].notes
    );
    assert!(!s.backup.exists(), "this change's own backup is removed");
}

#[test]
fn a_file_another_change_holds_is_left_busy() {
    let s = scene();
    std::fs::write(&s.target, ORIGINAL).expect("original in place");
    std::fs::write(&s.temp, OUTPUT).expect("temp");
    // A new change of the same file is running: it holds the file's lock.
    let newer = TargetLock::try_acquire(&s.journal, &s.target)
        .expect("io")
        .expect("free");
    let r = recover(&s.root).expect("read");
    assert!(r.recovered.is_empty(), "{r:?}");
    assert_eq!(r.pending.len(), 1);
    assert!(r.pending[0].reason.starts_with("busy"), "{r:?}");
    assert!(s.temp.exists() && s.backup.exists(), "nothing touched");
    assert!(
        TargetLock::try_acquire(&s.journal, &s.target)
            .expect("io")
            .is_none(),
        "still held"
    );
    drop(newer);
    assert_eq!(recover(&s.root).expect("now").recovered.len(), 1);
    assert!(!s.temp.exists());
}

#[test]
fn an_entry_whose_id_cannot_name_a_lock_stays_pending() {
    let s = scene();
    let mut planned = Line::new("../../escape", State::Planned);
    planned.kind = Some(TxnKind::InPlace);
    planned.path = Some(s.target.clone());
    planned.source = Some(s.target.clone());
    planned.temp = Some(s.temp.clone());
    s.journal.append(&planned).expect("planned");
    std::fs::write(&s.target, ORIGINAL).expect("original in place");
    let r = recover(&s.root).expect("the others still run");
    assert_eq!(r.recovered.len(), 1, "{r:?}");
    assert_eq!(r.pending.len(), 1, "{r:?}");
    assert!(
        r.pending[0].reason.contains("not a valid change id"),
        "{r:?}"
    );
    assert!(!s.root.join("escape.lock").exists());
}

/// A render record as the `verified` line carries it.
fn record() -> crate::txn::sidecar::Record {
    use crate::txn::sidecar::{BlockCounts, Record, RenderSummary};
    Record {
        request: sc_core::RenderRequest {
            gain_db: -2.0,
            ..sc_core::RenderRequest::default()
        },
        render: RenderSummary {
            frames_in: 7,
            frames_out: 7,
            trim_frames: Some(0),
            trim_requested_frames: Some(0),
            sample_rate_hz: 44_100,
            channels: 2,
            bits_out: 16,
            exact: false,
            dithered: true,
            samples_saturated: 0,
            pcm_blake3: "cc".into(),
            blocks: BlockCounts {
                carried: 0,
                patched: 0,
                edited: 0,
                replaced: 2,
                dropped: 0,
            },
            tags_added: false,
            tags_not_added: None,
            stale_loudness_tags: Vec::new(),
        },
    }
}

#[test]
fn a_sidecar_written_by_recovery_keeps_what_the_backup_lacks() {
    let s = scene();
    let backup_note = "backup: extended attribute not restored: com.example.x (refused)";
    // The run journaled a backup note, renamed, then crashed before its metadata step.
    let mut planned = Line::new("t", State::Planned);
    planned.sidecar = Some(true);
    s.journal.append(&planned).expect("sidecar wanted");
    let mut verified = Line::new("t", State::Verified);
    verified.record = Some(record());
    s.journal.append(&verified).expect("record");
    let mut backed = Line::new("t", State::BackedUp);
    backed.notes = vec![backup_note.to_owned()];
    s.journal.append(&backed).expect("backup note");
    std::fs::write(&s.target, OUTPUT).expect("output in place");
    s.journal
        .append(&Line::new("t", State::Renamed))
        .expect("renamed");

    let r = recover(&s.root).expect("recovered").recovered;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].outcome, Outcome::Completed);
    let doc: serde_json::Value = serde_json::from_slice(
        &std::fs::read(crate::txn::sidecar_path(&s.target)).expect("sidecar written"),
    )
    .expect("JSON");
    // A run without the crash lists the backup's notes, then the metadata step's (none here).
    assert_eq!(doc["metadata"]["notes"], serde_json::json!([backup_note]));
    let e = s.journal.entry("t").expect("read").expect("entry");
    assert_eq!(e.notes, vec![backup_note.to_owned()], "journaled once");
}
