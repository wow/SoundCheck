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

#[test]
fn transactions_work_on_exfat_and_fat32() {
    if !enabled() {
        eprintln!("skipped: set SC_DISK_IMAGES=1 on macOS to run");
        return;
    }
    on_volume("ExFAT", true);
    on_volume("MS-DOS FAT32", false);
}
