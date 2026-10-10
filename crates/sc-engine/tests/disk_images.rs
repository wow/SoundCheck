//! Exports on FAT32 and exFAT volumes, where a file's change time is its modification time and
//! inodes are slot numbers a remount can hand to another file: an export cancelled at Written,
//! or a change by `apply_file`, followed by a remount, must not let the next export find a stale
//! analysis and apply its gain twice. 200 MB disk images made, mounted, remounted and detached
//! with `hdiutil`. macOS only, opt-in with `SC_DISK_IMAGES=1` (skipped otherwise); backups,
//! cache and edits live in a temp folder on the startup disk.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::export::*;
use sc_core::export::BatchMode;
use sc_engine::{
    Analyzer, ApplyOptions, ApplyRequest, CancelToken, EngineEvent, apply_file, run_batch,
};

/// A mounted disk image, detached when dropped.
struct Mounted {
    _dir: tempfile::TempDir,
    image: PathBuf,
    mount: PathBuf,
}

fn hdiutil(args: &[&str]) -> bool {
    Command::new("hdiutil")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

impl Mounted {
    fn new(fs: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let image = dir.path().join("t.dmg");
        let mount = dir.path().join("mnt");
        let image_s = image.to_str().expect("UTF-8");
        assert!(
            hdiutil(&[
                "create", "-size", "200m", "-fs", fs, "-volname", "T", image_s
            ]),
            "create {fs}"
        );
        std::fs::create_dir(&mount).expect("mount point");
        let m = Self {
            _dir: dir,
            image,
            mount,
        };
        m.attach();
        m
    }

    fn attach(&self) {
        let (image, mount) = (
            self.image.to_str().expect("UTF-8"),
            self.mount.to_str().expect("UTF-8"),
        );
        assert!(
            hdiutil(&["attach", "-nobrowse", "-mountpoint", mount, image]),
            "attach"
        );
    }

    /// Detaches and attaches the image again at the same place.
    fn remount(&self) {
        assert!(
            hdiutil(&["detach", self.mount.to_str().expect("UTF-8")]),
            "detach"
        );
        self.attach();
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

/// S-P95 of `path` measured afresh, without the cache.
fn measured_p95(path: &Path) -> f64 {
    Analyzer::load(common::loudness_only(), None, CancelToken::new())
        .expect("analyzer")
        .analyze(path)
        .expect("analysed")
        .record
        .loudness
        .short_term_p95
        .expect("S-P95")
        .0
}

/// Exports `path` in Library mode, cancelling at Written when `cancel_at_written`.
fn export(lib: &Lib, path: &Path, cancel_at_written: bool) {
    let s = settings(
        lib,
        common::loudness_only(),
        process(lib, BatchMode::Library, None),
    );
    let cancel = CancelToken::new();
    let mut last = None;
    run_batch(&batch(&[path.to_path_buf()]), &s, &cancel, &mut |e| {
        if cancel_at_written && matches!(e, EngineEvent::Written { .. }) {
            cancel.cancel();
        }
        if matches!(e, EngineEvent::Done { .. } | EngineEvent::Failed { .. }) {
            last = Some(format!("{e:?}").chars().take(200).collect::<String>());
        }
    })
    .expect("batch starts");
    assert!(
        last.as_deref().is_some_and(|l| l.starts_with("Done")),
        "{last:?}"
    );
}

fn on_volume(fs: &str) {
    let vol = Mounted::new(fs);
    let lib = Lib::new();

    // An export cancelled at Written, a remount, another export: at the target, not past it.
    let a = vol.mount.join("a.wav");
    std::fs::copy(common::tone_wav(&lib.music(), "a.wav", 3.0, 0.1), &a).expect("fixture");
    export(&lib, &a, true);
    vol.remount();
    export(&lib, &a, false);
    let p95 = measured_p95(&a);
    assert!((p95 + 11.0).abs() < 0.05, "{fs}: cancelled export, {p95}");

    // The case the cache key alone cannot catch here: an export cancelled at Written leaves
    // the original's analysis, a change that does not analyse writes a file of the same length
    // with the time kept, which the volume may place where the original was (the same inode
    // after a remount, the change time equal to the modification time). The next export must
    // measure that file, not serve the original's analysis.
    let b = vol.mount.join("b.wav");
    std::fs::copy(common::tone_wav(&lib.music(), "b.wav", 3.0, 0.1), &b).expect("fixture");
    vol.remount();
    export(&lib, &b, true);
    let opts = ApplyOptions {
        cache: Some(lib.cache()),
        ..ApplyOptions::in_place(lib.backups())
    };
    let request = ApplyRequest {
        gain_db: -3.0,
        ..ApplyRequest::default()
    };
    apply_file(&b, &request, &opts, &CancelToken::new()).expect("applied");
    vol.remount();
    export(&lib, &b, false);
    let p95 = measured_p95(&b);
    assert!(
        (p95 + 11.0).abs() < 0.05,
        "{fs}: applied then exported, {p95}"
    );
}

#[test]
fn exports_on_fat32_never_meet_a_stale_analysis() {
    if enabled() {
        on_volume("MS-DOS FAT32");
    }
}

#[test]
fn exports_on_exfat_never_meet_a_stale_analysis() {
    if enabled() {
        on_volume("ExFAT");
    }
}
