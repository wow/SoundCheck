//! Transactions on exFAT and FAT32 volumes (no exclusive rename, no hard links, extended
//! attributes in `AppleDouble` files): 200 MB disk images (the 64 MiB free-space margin must
//! fit) made and mounted with `hdiutil`, detached and deleted afterwards. macOS only, opt-in
//! with `SC_DISK_IMAGES=1` (skipped when `hdiutil` is absent); needs the `crash-test` feature
//! for the placeholder crash case.

mod common;

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

use common::{Library, assert_no_temps, blake3_of, files_under, wav};
use sc_core::{Error, RenderRequest};
use sc_io::txn::{self, BACKUP_ROOT_ENV, CRASH_ENV, Outcome, TxnOptions, sidecar_path};

/// A mounted disk image, detached when dropped (the image file goes with its temp folder).
struct Mounted {
    _dir: tempfile::TempDir,
    mount: PathBuf,
}

impl Mounted {
    /// A 200 MB image formatted `fs` (`hdiutil -fs`), mounted without showing it in the Finder.
    fn new(fs: &str) -> Option<Self> {
        let dir = tempfile::tempdir().expect("temp dir");
        let image = dir.path().join("t.dmg");
        let mount = dir.path().join("mnt");
        let ok = |args: &[&str]| {
            Command::new("hdiutil")
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        let image_s = image.to_str().expect("UTF-8");
        if !ok(&[
            "create", "-size", "200m", "-fs", fs, "-volname", "T", image_s,
        ]) {
            return None;
        }
        std::fs::create_dir(&mount).expect("mount point");
        let mount_s = mount.to_str().expect("UTF-8");
        assert!(
            ok(&["attach", "-nobrowse", "-mountpoint", mount_s, image_s]),
            "attach {fs}"
        );
        Some(Self { _dir: dir, mount })
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-force"])
            .arg(&self.mount)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn enabled() -> bool {
    std::env::var_os("SC_DISK_IMAGES").is_some_and(|v| v == "1")
        && cfg!(target_os = "macos")
        && Command::new("hdiutil")
            .arg("help")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
}

fn req() -> RenderRequest {
    RenderRequest {
        gain_db: -2.0,
        ..RenderRequest::default()
    }
}

/// `placeholder`: whether the volume lacks an exclusive rename (exFAT; FAT32 has one), so a
/// copy goes through a placeholder.
fn on_volume(fs: &str, placeholder: bool) {
    let Some(vol) = Mounted::new(fs) else {
        panic!("hdiutil could not make a {fs} image");
    };
    let cancel = AtomicBool::new(false);
    // Copy from the startup disk onto the volume: no exclusive rename there.
    let lib = Library::new();
    let src = lib.add("a.wav", &wav(3000, 41));
    let out = vol.mount.join("out");
    let opts = TxnOptions::new(&lib.backups);
    let r = txn::apply_to_folder(&src, &out, &req(), &opts, &cancel).expect("copied");
    assert_eq!(blake3_of(&r.output), r.output_blake3, "{fs}");
    assert!(matches!(
        txn::apply_to_folder(&src, &out, &req(), &opts, &cancel),
        Err(Error::AlreadyExists { .. })
    ));

    // In place on the volume, with the backup root on it too.
    let path = vol.mount.join("b.wav");
    std::fs::write(&path, wav(3000, 42)).expect("fixture");
    xattr::set(&path, "com.apple.metadata:_kMDItemUserTags", b"tag").expect("xattr");
    let original = blake3_of(&path);
    let backups = vol.mount.join("Backups");
    let r =
        txn::apply_in_place(&path, &req(), &TxnOptions::new(&backups), &cancel).expect("in place");
    assert_eq!(blake3_of(&r.backup.expect("backup")), original, "{fs}");
    assert_eq!(
        xattr::get(&path, "com.apple.metadata:_kMDItemUserTags").expect("read"),
        Some(b"tag".to_vec()),
        "{fs}"
    );
    assert!(sidecar_path(&path).exists());
    txn::undo(&path, &backups).expect("undone");
    assert_eq!(blake3_of(&path), original, "{fs}");
    assert_no_temps(&vol.mount);

    if !placeholder {
        return;
    }
    // A crash between the placeholder and the rename leaves an empty file; recovery removes it.
    let target_dir = vol.mount.join("crash");
    let status = Command::new(env!("CARGO_BIN_EXE_txn_apply"))
        .arg("folder")
        .args([&src, &lib.backups, &target_dir])
        .env(CRASH_ENV, "placeholder")
        .env_remove(BACKUP_ROOT_ENV)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("txn_apply runs");
    assert_eq!(status.signal(), Some(6), "{fs}: {status}");
    assert_eq!(
        std::fs::metadata(target_dir.join("a.wav"))
            .expect("placeholder")
            .len(),
        0
    );
    let report = txn::recover(&lib.backups).expect("recovery");
    assert_eq!(report.recovered.len(), 1, "{fs}: {report:?}");
    assert_eq!(report.recovered[0].outcome, Outcome::RolledBack);
    assert!(
        !target_dir.join("a.wav").exists(),
        "{fs}: placeholder removed"
    );
    assert_no_temps(&vol.mount);
    let left: Vec<_> = files_under(&target_dir);
    assert!(left.iter().all(|p| is_apple_double(p)), "{fs}: {left:?}");
}

/// `AppleDouble` companions (`._name`) the system keeps for extended attributes on FAT volumes.
fn is_apple_double(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("._"))
}

/// On FAT32 (macOS 15 mounts it through `FSKit`) the folder lists a Turkish name decomposed,
/// while asking the open file for its path answers with whichever spelling first reached it
/// since the mount. The rekordbox location is spelled from the listing: a path typed
/// precomposed and one typed decomposed give the same bytes, the listing's.
#[test]
fn rekordbox_locations_follow_the_listing_on_fat32() {
    use sc_io::rekordbox::{Speller, location};
    use unicode_normalization::UnicodeNormalization;
    if !enabled() {
        eprintln!("skipped: set SC_DISK_IMAGES=1 on macOS to run");
        return;
    }
    let Some(vol) = Mounted::new("MS-DOS FAT32") else {
        panic!("hdiutil could not make a FAT32 image");
    };
    let nfc = "Şarkı Çiçek Ğüzel.wav";
    let nfd: String = nfc.nfd().collect();
    std::fs::write(vol.mount.join(nfc), b"x").expect("write");
    let listed = std::fs::read_dir(&vol.mount)
        .expect("list")
        .map(|e| e.expect("entry").file_name())
        .find(|n| n.to_string_lossy().nfc().eq(nfc.chars()))
        .expect("listed");
    // The decomposed spelling reaches the file first, then the precomposed one, each through a
    // fresh speller (no listing shared between them).
    let decomposed = Speller::new().spell(&vol.mount.join(&nfd));
    let precomposed = Speller::new().spell(&vol.mount.join(nfc));
    assert_eq!(location(&decomposed), location(&precomposed));
    assert_eq!(precomposed.file_name(), Some(listed.as_os_str()));
}

#[test]
fn transactions_work_on_exfat_and_fat32() {
    if !enabled() {
        eprintln!("skipped: set SC_DISK_IMAGES=1 on macOS to run");
        return;
    }
    on_volume("ExFAT", true);
    on_volume("MS-DOS FAT32", false);
}

fn hdiutil(args: &[&std::ffi::OsStr]) -> bool {
    Command::new("hdiutil")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Detaches whatever is mounted at the path when dropped.
struct Detach<'a>(&'a Path);

impl Drop for Detach<'_> {
    fn drop(&mut self) {
        let _ = hdiutil(&["detach".as_ref(), "-force".as_ref(), self.0.as_os_str()]);
    }
}

fn make_image(image: &Path) {
    assert!(hdiutil(&[
        "create".as_ref(),
        "-size".as_ref(),
        "200m".as_ref(),
        "-fs".as_ref(),
        "ExFAT".as_ref(),
        "-volname".as_ref(),
        "T".as_ref(),
        image.as_os_str()
    ]));
}

fn attach(image: &Path, mount: &Path) {
    assert!(hdiutil(&[
        "attach".as_ref(),
        "-nobrowse".as_ref(),
        "-mountpoint".as_ref(),
        mount.as_os_str(),
        image.as_os_str()
    ]));
}

fn detach(mount: &Path) {
    assert!(hdiutil(&["detach".as_ref(), mount.as_os_str()]));
}

/// Changes `file` in place with the backups on the startup disk, crashing at `point`.
fn crash_in_place(file: &Path, backups: &Path, point: &str) {
    let status = Command::new(env!("CARGO_BIN_EXE_txn_apply"))
        .arg("apply")
        .args([file, backups])
        .env(CRASH_ENV, point)
        .env_remove(BACKUP_ROOT_ENV)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("txn_apply runs");
    assert_eq!(status.signal(), Some(6), "{point}: {status}");
}

#[test]
fn an_empty_mount_folder_or_another_volume_keeps_recovery_pending() {
    if !enabled() {
        eprintln!("skipped: set SC_DISK_IMAGES=1 on macOS to run");
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let (image_a, image_b) = (dir.path().join("a.dmg"), dir.path().join("b.dmg"));
    make_image(&image_a);
    make_image(&image_b);
    // Mounted at a folder of its own (not under /Volumes), which stays when it is detached.
    let mount = dir.path().join("mnt");
    std::fs::create_dir(&mount).expect("mount folder");
    let _cleanup = Detach(&mount);
    let lib = Library::new();
    let file = mount.join("b.wav");
    attach(&image_a, &mount);
    std::fs::write(&file, wav(3000, 51)).expect("fixture");
    let original = blake3_of(&file);
    detach(&mount);
    // Another stick with the same name and a file at the same path.
    attach(&image_b, &mount);
    std::fs::write(&file, wav(3000, 52)).expect("other file");
    let other = blake3_of(&file);
    detach(&mount);

    for (point, outcome) in [
        ("backed_up", Outcome::RolledBack),
        ("renamed", Outcome::Completed),
    ] {
        attach(&image_a, &mount);
        crash_in_place(&file, &lib.backups, point);
        detach(&mount);
        assert!(mount.is_dir(), "the empty mount folder stays");
        let r = txn::recover(&lib.backups).expect("recovery");
        assert!(r.recovered.is_empty(), "{point}, unmounted: {r:?}");
        assert_eq!(r.pending.len(), 1, "{point}: {r:?}");

        attach(&image_b, &mount);
        let r = txn::recover(&lib.backups).expect("recovery");
        assert!(r.recovered.is_empty(), "{point}, other stick: {r:?}");
        assert_eq!(
            blake3_of(&file),
            other,
            "{point}: the other stick's file is untouched"
        );
        assert!(
            !sidecar_path(&file).exists(),
            "{point}: no sidecar on the other stick"
        );
        detach(&mount);

        attach(&image_a, &mount);
        let r = txn::recover(&lib.backups).expect("recovery");
        assert_eq!(r.recovered.len(), 1, "{point}: {r:?}");
        assert_eq!(r.recovered[0].outcome, outcome, "{point}");
        assert_no_temps(&mount);
        if outcome == Outcome::Completed {
            assert!(sidecar_path(&file).exists());
            txn::undo(&file, &lib.backups).expect("undo");
        }
        assert_eq!(blake3_of(&file), original, "{point}");
        detach(&mount);
    }
}
