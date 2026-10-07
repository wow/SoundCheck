//! Unit tests of `volume.rs`: the naming rule, the startup disk's firmlinked paths, relative
//! paths and the rekordbox export check.

use super::*;

fn named(mount: &str, startup: Option<&str>) -> String {
    volume_name(Path::new(mount), || startup.map(str::to_owned))
}

#[test]
fn volumes_are_named_by_their_mount_point() {
    assert_eq!(named("/Volumes/DJ USB", None), "DJ USB");
    assert_eq!(named("/Volumes/a:b", None), "a:b");
    assert_eq!(named("/mnt/music", None), "music");
    assert_eq!(named("/Volumes/x/nested", None), "nested");
    if cfg!(target_os = "macos") {
        assert_eq!(named("/", Some("Macintosh HD")), "Macintosh HD");
        assert_eq!(named("/System/Volumes/Data", Some("Work")), "Work");
        assert_eq!(named("/", None), STARTUP_DISK_NAME);
    } else {
        assert_eq!(named("/", None), "Root");
    }
}

#[test]
fn paths_on_the_startup_disk_are_relative_to_the_root() {
    let data = Path::new("/System/Volumes/Data");
    assert_eq!(
        root_for(data, Path::new("/Users/me/Music/a.wav")),
        PathBuf::from("/")
    );
    assert_eq!(
        root_for(data, Path::new("/System/Volumes/Data/x/a.wav")),
        data.to_path_buf()
    );
    let usb = Path::new("/Volumes/USB");
    assert_eq!(root_for(usb, Path::new("/Volumes/USB/a.wav")), usb);
    let v = Volume {
        root: PathBuf::from("/Volumes/USB"),
        name: "USB".into(),
        id: 7,
        free_bytes: 0,
    };
    assert_eq!(
        v.relative(Path::new("/Volumes/USB/Crates/a.wav")),
        Some(Path::new("Crates/a.wav"))
    );
    assert_eq!(v.relative(Path::new("/Users/a.wav")), None);
}

#[test]
fn a_pioneer_folder_at_the_root_marks_a_rekordbox_export() {
    let dir = tempfile::tempdir().expect("temp dir");
    let v = Volume {
        root: dir.path().to_path_buf(),
        name: "USB".into(),
        id: 1,
        free_bytes: 0,
    };
    assert!(!v.is_rekordbox_export());
    std::fs::write(dir.path().join(REKORDBOX_EXPORT_FOLDER), b"a file").expect("file");
    assert!(
        !v.is_rekordbox_export(),
        "a file named PIONEER is not the folder"
    );
    std::fs::remove_file(dir.path().join(REKORDBOX_EXPORT_FOLDER)).expect("rm");
    std::fs::create_dir(dir.path().join(REKORDBOX_EXPORT_FOLDER)).expect("folder");
    assert!(v.is_rekordbox_export());
}

#[test]
fn the_system_reports_a_volume_with_free_space() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().canonicalize().expect("resolves");
    let v = SystemVolumes.volume_of(&path).expect("volume");
    assert!(v.free_bytes > 0);
    assert!(!v.name.is_empty(), "a name");
    assert!(path.starts_with(&v.root), "{path:?} under {:?}", v.root);
    let file_meta = std::fs::metadata(&path).expect("metadata");
    assert_eq!(v.id, file_meta.dev());
}

#[test]
fn a_command_past_its_deadline_is_killed_and_gives_nothing() {
    let start = std::time::Instant::now();
    let mut slow = std::process::Command::new("/bin/sleep");
    slow.arg("10");
    assert_eq!(
        run_with_deadline(slow, std::time::Duration::from_millis(200)),
        None
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "killed promptly"
    );

    let mut quick = std::process::Command::new("/bin/echo");
    quick.arg("VolumeUUID");
    assert_eq!(
        run_with_deadline(quick, std::time::Duration::from_secs(5)).as_deref(),
        Some(&b"VolumeUUID\n"[..])
    );

    // A command that fails gives nothing either.
    let failing = std::process::Command::new("/usr/bin/false");
    assert_eq!(
        run_with_deadline(failing, std::time::Duration::from_secs(5)),
        None
    );
}
