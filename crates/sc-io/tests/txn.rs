//! Write transactions end to end in temp folders: in place (WAV, AIFF, FLAC) with backup,
//! sidecar and the original's metadata kept; copy to a folder; every refusal leaving the file
//! untouched and nothing behind; cancel during the render; undo (twice, after a second change,
//! after an outside edit). The crash matrix is `tests/crash.rs`.

mod common;

use std::fs::FileTimes;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use common::{
    FakeVolumes, Library, aiff, assert_no_temps, blake3_of, files_under, flac, temps_under, wav,
};
use sc_core::{Error, InPlaceRefusal, RenderRequest, TagEdit};
use sc_io::txn::{
    self, SidecarAfterUndo, State, Transaction, TxnKind, TxnOptions, hex, journal_entries,
    sidecar_path,
};

/// The extended attribute the metadata tests set: a Finder tag on macOS, a user attribute on
/// Linux (where `com.apple.*` names are not allowed).
#[cfg(target_os = "macos")]
const TAG_XATTR: &str = "com.apple.metadata:_kMDItemUserTags";
#[cfg(not(target_os = "macos"))]
const TAG_XATTR: &str = "user.soundcheck.test";

/// A binary property list holding the Finder tag "Red\n6".
const TAG_VALUE: &[u8] = b"bplist00\xa1\x01URed\n6\x08\x0a\x00\x00\x00\x00\x00\x00\x01\x01\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x10";

static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

fn gain(gain_db: f64) -> RenderRequest {
    RenderRequest {
        gain_db,
        ..RenderRequest::default()
    }
}

fn opts(lib: &Library) -> TxnOptions {
    TxnOptions::new(&lib.backups)
}

/// 2001-09-09 01:46:40 UTC and a later instant, as file times.
fn old_times() -> (SystemTime, SystemTime) {
    let created = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    (created, created + Duration::from_hours(24))
}

/// Gives the file at `path` a Finder tag, mode 0640, an old modification and creation date.
fn decorate(path: &Path) {
    xattr::set(path, TAG_XATTR, TAG_VALUE).expect("xattr set");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)).expect("mode");
    let (created, modified) = old_times();
    let times = FileTimes::new().set_modified(modified);
    #[cfg(target_os = "macos")]
    let times = {
        use std::os::macos::fs::FileTimesExt;
        times.set_created(created)
    };
    #[cfg(not(target_os = "macos"))]
    let _ = created;
    let f = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open");
    f.set_times(times).expect("times");
}

fn assert_decorated(path: &Path) {
    let value = xattr::get(path, TAG_XATTR).expect("xattr read");
    assert_eq!(
        value.as_deref(),
        Some(TAG_VALUE),
        "Finder tag of {}",
        path.display()
    );
    let meta = std::fs::metadata(path).expect("metadata");
    assert_eq!(
        meta.permissions().mode() & 0o7777,
        0o640,
        "mode of {}",
        path.display()
    );
    let (created, modified) = old_times();
    assert_eq!(
        meta.modified().expect("mtime"),
        modified,
        "mtime of {}",
        path.display()
    );
    if cfg!(target_os = "macos") {
        assert_eq!(
            meta.created().expect("birth time"),
            created,
            "creation date"
        );
    }
}

/// Runs an in-place change of `name` built by `build` and checks every promise of the
/// transaction.
fn in_place_round_trip(name: &str, bytes: &[u8]) {
    let lib = Library::new();
    let path = lib.add(name, bytes);
    decorate(&path);
    let original = blake3_of(&path);
    let req = RenderRequest {
        tag_edits: vec![TagEdit {
            label: if Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("flac"))
            {
                "SOUNDCHECK".into()
            } else {
                "TXXX:SOUNDCHECK".into()
            },
            value: "test".into(),
        }],
        ..gain(-3.0)
    };
    let report = txn::apply_in_place(&path, &req, &opts(&lib), &NOT_CANCELLED).expect("applied");
    assert_eq!(report.kind, TxnKind::InPlace);
    assert_eq!(report.original_blake3, original);
    assert_eq!(
        report.output_blake3,
        blake3_of(&path),
        "the file is the output"
    );
    assert_ne!(report.output_blake3, original);
    assert_eq!(report.render.frames_out, 6000);

    let backup = report.backup.clone().expect("a backup");
    assert!(backup.starts_with(&lib.backups));
    assert_eq!(blake3_of(&backup), original, "the backup is the original");
    assert!(
        backup.ends_with(Path::new("music").join(name)),
        "{}",
        backup.display()
    );
    assert_decorated(&path);
    assert_decorated(&backup);

    let sidecar = report.sidecar.clone().expect("a sidecar");
    assert_eq!(sidecar, sidecar_path(&path));
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).expect("sidecar")).expect("JSON");
    assert_eq!(doc["schema"], 1);
    assert_eq!(doc["file"], name);
    assert_eq!(doc["original"]["blake3"], hex(&original));
    assert_eq!(doc["output"]["blake3"], hex(&report.output_blake3));
    assert_eq!(doc["request"]["gain_db"], -3.0);
    assert_eq!(doc["backup"], backup.to_str().expect("UTF-8"));
    assert_eq!(doc["render"]["frames_out"], 6000);
    assert_no_temps(lib.dir.path());

    let entries = journal_entries(&lib.backups).expect("journal");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].state, State::Done);
    assert!(report.timings.iter().map(|(s, _)| *s).eq([
        State::Planned,
        State::TempWritten,
        State::Verified,
        State::BackedUp,
        State::Renamed,
        State::MetadataDone,
        State::Done
    ]));

    let undone = txn::undo(&path, &lib.backups).expect("undone");
    assert_eq!(blake3_of(&path), original, "undo restores the original");
    assert_eq!(undone.restored_blake3, original);
    assert_eq!(undone.sidecar, SidecarAfterUndo::Removed);
    assert!(!sidecar.exists());
    assert_decorated(&path);
    assert!(backup.exists(), "the backup is kept");
    assert_no_temps(lib.dir.path());
    match txn::undo(&path, &lib.backups) {
        Err(Error::NothingToUndo { .. }) => {}
        other => panic!("second undo: {other:?}"),
    }
}

#[test]
fn a_wav_is_changed_in_place_backed_up_and_undone() {
    in_place_round_trip("a.wav", &wav(6000, 1));
}

#[test]
fn an_aiff_is_changed_in_place_backed_up_and_undone() {
    in_place_round_trip("b.aiff", &aiff(6000, 2));
}

#[test]
fn a_flac_is_changed_in_place_backed_up_and_undone() {
    in_place_round_trip("c.flac", &flac(6000, 3));
}

#[test]
fn copy_to_a_folder_leaves_the_source_and_never_overwrites() {
    let lib = Library::new();
    let path = lib.add("a.wav", &wav(5000, 4));
    decorate(&path);
    let original = blake3_of(&path);
    let out = lib.dir.path().join("out");
    let report = txn::apply_to_folder(&path, &out, &gain(-1.5), &opts(&lib), &NOT_CANCELLED)
        .expect("copied");
    assert_eq!(report.kind, TxnKind::ToFolder);
    assert_eq!(blake3_of(&path), original, "the source is untouched");
    assert_eq!(
        report.output,
        out.canonicalize().expect("out").join("a.wav")
    );
    assert_eq!(blake3_of(&report.output), report.output_blake3);
    assert!(report.backup.is_none());
    assert!(sidecar_path(&report.output).exists());
    assert!(!sidecar_path(&path).exists());
    assert_decorated(&report.output);
    match txn::apply_to_folder(&path, &out, &gain(-1.5), &opts(&lib), &NOT_CANCELLED) {
        Err(Error::AlreadyExists { path: p }) => assert_eq!(p, report.output),
        other => panic!("second copy: {other:?}"),
    }
    assert_no_temps(lib.dir.path());
}

/// Asserts that changing `path` in place with `tx` fails as `is` says, and that the file is
/// untouched and nothing was written anywhere.
fn assert_refused(lib: &Library, tx: &Transaction<'_>, path: &Path, is: impl Fn(&Error) -> bool) {
    let before = std::fs::read(path).ok();
    let music_before = files_under(&lib.music);
    let had_backup_root = lib.backups.exists();
    match tx.apply_in_place(path, &gain(-2.0), &opts(lib), &NOT_CANCELLED) {
        Ok(_) => panic!("{} was not refused", path.display()),
        Err(e) => assert!(is(&e), "unexpected error: {e}"),
    }
    assert_eq!(std::fs::read(path).ok(), before, "the file changed");
    assert_eq!(files_under(&lib.music), music_before, "files appeared");
    let backups: Vec<_> = files_under(&lib.backups)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e != "jsonl" && e != "lock"))
        .collect();
    assert!(backups.is_empty(), "backups written: {backups:?}");
    if !had_backup_root {
        assert!(!lib.backups.exists(), "a refusal created the backup root");
    }
}

fn refused_as(want: InPlaceRefusal) -> impl Fn(&Error) -> bool {
    move |e| matches!(e, Error::InPlaceRefused { reason, .. } if *reason == want)
}

#[test]
fn symlinks_hard_links_and_read_only_files_are_refused_in_place() {
    let lib = Library::new();
    let tx = Transaction::default();
    let target = lib.add("real.wav", &wav(2000, 5));
    let link = lib.music.join("link.wav");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    assert_refused(&lib, &tx, &link, refused_as(InPlaceRefusal::Symlink));
    std::fs::remove_file(&link).expect("unlink");

    let hard = lib.music.join("hard.wav");
    std::fs::hard_link(&target, &hard).expect("hard link");
    assert_refused(
        &lib,
        &tx,
        &target,
        refused_as(InPlaceRefusal::HardLinked { links: 2 }),
    );
    std::fs::remove_file(&hard).expect("unlink");

    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).expect("mode");
    assert_refused(&lib, &tx, &target, refused_as(InPlaceRefusal::ReadOnlyFile));
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).expect("mode");

    std::fs::set_permissions(&lib.music, std::fs::Permissions::from_mode(0o555)).expect("mode");
    assert_refused(
        &lib,
        &tx,
        &target,
        refused_as(InPlaceRefusal::ReadOnlyFolder),
    );
    std::fs::set_permissions(&lib.music, std::fs::Permissions::from_mode(0o755)).expect("mode");
}

#[cfg(target_os = "macos")]
#[test]
fn finder_locked_files_and_files_with_an_acl_are_refused_in_place() {
    let lib = Library::new();
    let tx = Transaction::default();
    let path = lib.add("locked.wav", &wav(2000, 6));
    let run = |args: &[&str]| {
        let ok = std::process::Command::new(args[0])
            .args(&args[1..])
            .status()
            .expect("command runs")
            .success();
        assert!(ok, "{args:?} failed");
    };
    let p = path.to_str().expect("UTF-8");
    run(&["chflags", "uchg", p]);
    assert_refused(&lib, &tx, &path, refused_as(InPlaceRefusal::FinderLocked));
    run(&["chflags", "nouchg", p]);

    run(&["chmod", "+a", "everyone deny delete", p]);
    assert_refused(&lib, &tx, &path, refused_as(InPlaceRefusal::HasAcl));
    run(&["chmod", "-N", p]);
    tx.apply_in_place(&path, &gain(-2.0), &opts(&lib), &NOT_CANCELLED)
        .expect("accepted once unlocked and without an ACL");
}

#[test]
fn rekordbox_exports_and_full_volumes_are_refused() {
    let lib = Library::new();
    let path = lib.add("a.wav", &wav(2000, 7));
    std::fs::create_dir(lib.dir.path().join("PIONEER")).expect("PIONEER");
    let usb = FakeVolumes {
        root: lib.dir.path().to_path_buf(),
        free_bytes: u64::MAX,
    };
    let tx = Transaction::with_volumes(&usb);
    assert_refused(&lib, &tx, &path, |e| {
        matches!(e, Error::RekordboxUsbExport { .. })
    });
    let out = lib.dir.path().join("out").join("nested");
    match tx.apply_to_folder(&path, &out, &gain(-2.0), &opts(&lib), &NOT_CANCELLED) {
        Err(Error::RekordboxUsbExport { .. }) => {}
        other => panic!("copy onto a USB export: {other:?}"),
    }
    assert!(!lib.dir.path().join("out").exists(), "no folder created");
    assert!(!lib.backups.exists(), "no backup root created");

    let elsewhere = tempfile::tempdir().expect("temp dir");
    let full = FakeVolumes {
        root: elsewhere.path().to_path_buf(),
        free_bytes: 64 << 20,
    };
    let tx = Transaction::with_volumes(&full);
    assert_refused(&lib, &tx, &path, |e| {
        matches!(e, Error::NoSpace { needed_bytes, free_bytes, .. }
            if *free_bytes == 64 << 20 && *needed_bytes > *free_bytes)
    });
}

#[test]
fn other_formats_are_refused() {
    let lib = Library::new();
    let path = lib.add(
        "a.mp3",
        b"ID3\x04\x00\x00\x00\x00\x00\x00not a FLAC stream at all",
    );
    assert_refused(&lib, &Transaction::default(), &path, |e| {
        matches!(e, Error::UnsupportedFormat { .. })
    });
    let text = lib.add("notes.wav", b"just some text, not audio");
    assert_refused(&lib, &Transaction::default(), &text, |e| {
        matches!(e, Error::UnsupportedFormat { .. })
    });
}

#[test]
fn cancelling_during_the_render_leaves_the_original_and_no_temp() {
    let lib = Library::new();
    // 60 s: the render writes for a while after the temp file first has bytes.
    let path = lib.add("long.wav", &wav(44_100 * 60, 8));
    let original = blake3_of(&path);
    let cancel = AtomicBool::new(false);
    let result = std::thread::scope(|s| {
        s.spawn(|| {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while std::time::Instant::now() < deadline {
                let started = temps_under(&lib.music)
                    .iter()
                    .any(|t| std::fs::metadata(t).is_ok_and(|m| m.len() > 0));
                if started {
                    cancel.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        });
        txn::apply_in_place(&path, &gain(-2.0), &opts(&lib), &cancel)
    });
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert_eq!(blake3_of(&path), original);
    assert_no_temps(lib.dir.path());
    let entries = journal_entries(&lib.backups).expect("journal");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].state, State::Failed);
    assert_eq!(entries[0].reached, State::Planned);
    assert!(
        txn::recover(&lib.backups)
            .expect("recover")
            .recovered
            .is_empty(),
        "nothing to recover"
    );
}

#[test]
fn undo_walks_back_through_two_changes_and_refuses_an_edited_file() {
    let lib = Library::new();
    let path = lib.add("a.wav", &wav(4000, 9));
    let original = blake3_of(&path);
    let first = txn::apply_in_place(&path, &gain(-1.0), &opts(&lib), &NOT_CANCELLED).expect("1");
    let second = txn::apply_in_place(&path, &gain(-2.0), &opts(&lib), &NOT_CANCELLED).expect("2");
    assert_ne!(first.backup, second.backup, "backups are never overwritten");

    let undone = txn::undo(&path, &lib.backups).expect("undo 2");
    assert_eq!(undone.undone, second.txn);
    assert_eq!(
        undone.earlier_changes, 1,
        "the first change is still in effect"
    );
    assert_eq!(blake3_of(&path), first.output_blake3);
    assert_eq!(
        undone.sidecar,
        SidecarAfterUndo::Restored(sidecar_path(&path))
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(sidecar_path(&path)).expect("sidecar"))
            .expect("JSON");
    assert_eq!(doc["transaction"], first.txn.as_str());

    std::fs::write(&path, b"edited by another app").expect("edit");
    match txn::undo(&path, &lib.backups) {
        Err(Error::FileChanged { .. }) => {}
        other => panic!("undo of an edited file: {other:?}"),
    }
    // Put the first output back, as if the edit was reverted, and undo the first change.
    // (Backups themselves are refused as sources, so render a copy of the first backup.)
    let copy_dir = lib.dir.path().join("copy");
    std::fs::create_dir(&copy_dir).expect("folder");
    let copy = copy_dir.join("a.wav");
    std::fs::copy(first.backup.as_ref().expect("backup"), &copy).expect("copy out");
    match txn::apply_to_folder(
        first.backup.as_ref().expect("backup"),
        &lib.dir.path().join("unused"),
        &gain(-1.0),
        &opts(&lib),
        &NOT_CANCELLED,
    ) {
        Err(Error::InvalidArgument(why)) => assert!(why.contains("backup folder"), "{why}"),
        other => panic!("a backup as a source: {other:?}"),
    }
    std::fs::write(&path, b"").expect("truncate");
    let _ = txn::apply_to_folder(
        &copy,
        &lib.dir.path().join("unused"),
        &gain(-1.0),
        &opts(&lib),
        &NOT_CANCELLED,
    )
    .expect("render again");
    std::fs::copy(lib.dir.path().join("unused").join("a.wav"), &path).expect("restore output");
    assert_eq!(
        blake3_of(&path),
        first.output_blake3,
        "the render is deterministic"
    );
    let last = txn::undo(&path, &lib.backups).expect("undo 1");
    assert_eq!(last.earlier_changes, 0);
    assert_eq!(blake3_of(&path), original);
    assert!(!sidecar_path(&path).exists());
}

/// macOS: a 70,000-byte resource fork (reads of it are cut to the buffer unless sized first).
#[cfg(target_os = "macos")]
#[test]
fn a_large_resource_fork_survives_in_the_output_the_backup_and_the_undo() {
    let lib = Library::new();
    let path = lib.add("fork.wav", &wav(3000, 31));
    let fork: Vec<u8> = (0..70_000_u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(path.join("..namedfork/rsrc"), &fork).expect("resource fork written");
    let original = blake3_of(&path);
    let report =
        txn::apply_in_place(&path, &gain(-2.0), &opts(&lib), &NOT_CANCELLED).expect("applied");
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    let read_fork = |p: &Path| std::fs::read(p.join("..namedfork/rsrc")).expect("fork");
    assert_eq!(read_fork(&path).len(), fork.len());
    assert_eq!(read_fork(&path), fork, "output");
    let backup = report.backup.expect("backup");
    assert_eq!(read_fork(&backup), fork, "backup");
    txn::undo(&path, &lib.backups).expect("undone");
    assert_eq!(blake3_of(&path), original);
    assert_eq!(read_fork(&path), fork, "after undo");
}

/// macOS (case-insensitive, normalisation-insensitive volumes): one file reached under two
/// spellings is one file in the journal.
#[cfg(target_os = "macos")]
#[test]
fn a_file_is_known_by_its_name_on_disk_whatever_spelling_reaches_it() {
    let lib = Library::new();
    let path = lib.add("track.wav", &wav(3000, 32));
    let original = blake3_of(&path);
    let report = txn::apply_in_place(
        &lib.music.join("Track.wav"),
        &gain(-2.0),
        &opts(&lib),
        &NOT_CANCELLED,
    )
    .expect("applied");
    assert_eq!(report.output, path, "the name as stored");
    txn::undo(&lib.music.join("TRACK.WAV"), &lib.backups).expect("undone");
    assert_eq!(blake3_of(&path), original);

    let nfc = "Caf\u{e9}.wav";
    let nfd = "Cafe\u{301}.wav";
    let stored = lib.add(nfd, &wav(3000, 33));
    let original = blake3_of(&stored);
    let report = txn::apply_in_place(
        &lib.music.join(nfc),
        &gain(-2.0),
        &opts(&lib),
        &NOT_CANCELLED,
    )
    .expect("applied");
    assert_eq!(report.output, stored);
    assert!(sidecar_path(&stored).exists());
    txn::undo(&lib.music.join(nfc), &lib.backups).expect("undone through the other form");
    assert_eq!(blake3_of(&stored), original);
}

#[test]
fn a_fifo_is_refused_without_being_opened() {
    let lib = Library::new();
    let fifo = lib.music.join("x.wav");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    let started = std::time::Instant::now();
    let refused = txn::apply_in_place(&fifo, &gain(-2.0), &opts(&lib), &NOT_CANCELLED);
    assert!(
        matches!(refused, Err(Error::InvalidArgument(_))),
        "{refused:?}"
    );
    let out = lib.dir.path().join("out");
    let refused = txn::apply_to_folder(&fifo, &out, &gain(-2.0), &opts(&lib), &NOT_CANCELLED);
    assert!(
        matches!(refused, Err(Error::InvalidArgument(_))),
        "{refused:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(!lib.backups.exists() && !out.exists());
}

#[test]
fn a_copy_is_named_after_the_link_the_user_gave() {
    let lib = Library::new();
    let path = lib.add("track.wav", &wav(3000, 34));
    std::fs::hard_link(&path, lib.music.join("other.wav")).expect("hard link");
    let out = lib.dir.path().join("out");
    let r = txn::apply_to_folder(
        &lib.music.join("other.wav"),
        &out,
        &gain(-2.0),
        &opts(&lib),
        &NOT_CANCELLED,
    )
    .expect("copied");
    assert_eq!(r.output.file_name().expect("name"), "other.wav");
    if cfg!(target_os = "macos") {
        let r = txn::apply_to_folder(
            &lib.music.join("TRACK.WAV"),
            &out,
            &gain(-2.0),
            &opts(&lib),
            &NOT_CANCELLED,
        )
        .expect("copied");
        assert_eq!(
            r.output.file_name().expect("name"),
            "track.wav",
            "spelled as on disk"
        );
    }
}
