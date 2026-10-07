//! Unit tests of `fsx.rs`: dates, temp and numbered names, hashing copies and no-replace
//! renames.

use std::time::{Duration, UNIX_EPOCH};

use super::*;

#[test]
fn civil_dates_match_known_days() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
    // 2000-02-29 (a leap day in a century leap year) is day 11,016.
    assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    assert_eq!(civil_from_days(11_017), (2000, 3, 1));
    // 2026-10-07 is day 20,733.
    assert_eq!(civil_from_days(20_733), (2026, 10, 7));
}

#[test]
fn timestamps_are_rfc_3339_utc() {
    let t = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    assert_eq!(utc_date(t), "2001-09-09");
    assert_eq!(utc_timestamp(t), "2001-09-09T01:46:40Z");
    assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
}

#[test]
fn temp_names_are_hidden_marked_and_short() {
    let name = temp_name(OsStr::new("a.wav"), "1-2-3");
    assert_eq!(name, ".a.wav.soundcheck-tmp-1-2-3");
    // 200 three-byte characters: cut at a character boundary within the limit.
    let long = "\u{20ac}".repeat(200);
    let cut = temp_name(OsStr::new(&long), "x")
        .into_string()
        .expect("UTF-8");
    assert!(cut.len() <= TEMP_NAME_KEEP_BYTES + TEMP_MARKER.len() + 2);
    assert!(cut.starts_with(".\u{20ac}") && cut.ends_with(".soundcheck-tmp-x"));
}

#[test]
fn numbered_names_go_before_the_extension() {
    assert_eq!(
        numbered(Path::new("/b/a.wav"), 2),
        PathBuf::from("/b/a (2).wav")
    );
    assert_eq!(numbered(Path::new("/b/a"), 3), PathBuf::from("/b/a (3)"));
}

#[test]
fn hex_is_lowercase_and_64_digits() {
    let mut h = [0_u8; 32];
    h[0] = 0xAB;
    h[31] = 0x01;
    let s = hex(&h);
    assert_eq!(s.len(), 64);
    assert!(s.starts_with("ab00") && s.ends_with("0001"));
}

#[test]
fn ids_never_repeat() {
    let a = new_txn_id();
    let b = new_txn_id();
    assert_ne!(a, b);
    assert!(a.contains(&std::process::id().to_string()));
}

#[test]
fn copies_hash_what_they_write_and_never_overwrite() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("src");
    let bytes: Vec<u8> = (0..3_000_000_u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &bytes).expect("write");
    let dst = dir.path().join("dst");
    let (len, hash) = system_copy_hashed(&src, &dst).expect("copied");
    assert_eq!(len, bytes.len() as u64);
    assert_eq!(hash, *blake3::hash(&bytes).as_bytes());
    assert_eq!(std::fs::read(&dst).expect("read"), bytes);
    assert_eq!(hash_file(&dst).expect("hash"), (len, hash));
    match system_copy_hashed(&src, &dst) {
        Err(Error::AlreadyExists { path }) => assert_eq!(path, dst),
        other => panic!("copy over an existing file: {other:?}"),
    }
    assert_eq!(std::fs::read(&dst).expect("read"), bytes, "dst untouched");
}

#[test]
fn no_replace_renames_refuse_an_existing_target() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (a, b, c) = (
        dir.path().join("a"),
        dir.path().join("b"),
        dir.path().join("c"),
    );
    std::fs::write(&a, b"a").expect("write");
    std::fs::write(&b, b"b").expect("write");
    match rename_noreplace(&a, &b) {
        Err(Error::AlreadyExists { path }) => assert_eq!(path, b),
        other => panic!("rename over b: {other:?}"),
    }
    assert_eq!(std::fs::read(&b).expect("read"), b"b");
    rename_noreplace(&a, &c).expect("renamed");
    assert!(!a.exists());
    assert_eq!(std::fs::read(&c).expect("read"), b"a");
    assert!(remove_if_exists(&c).expect("removed"));
    assert!(!remove_if_exists(&c).expect("nothing to remove"));
}

#[test]
fn only_an_unsupported_full_sync_falls_back_to_fsync() {
    use rustix::io::Errno;
    for unsupported in [Errno::NOTSUP, Errno::OPNOTSUPP, Errno::INVAL, Errno::NOTTY] {
        let e = std::io::Error::from_raw_os_error(unsupported.raw_os_error());
        assert!(full_sync_unsupported(&e), "{e}");
    }
    for real in [Errno::IO, Errno::NOSPC, Errno::BADF, Errno::ROFS] {
        let e = std::io::Error::from_raw_os_error(real.raw_os_error());
        assert!(!full_sync_unsupported(&e), "{e}");
    }
    assert!(!full_sync_unsupported(&std::io::Error::other("no code")));
}

#[test]
fn the_local_date_is_a_calendar_date() {
    let t = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let d = local_date(t);
    // 2001-09-09 01:46:40 UTC is the 8th or 9th somewhere on Earth.
    assert!(d == "2001-09-09" || d == "2001-09-08", "{d}");
}

#[test]
fn syncs_succeed_on_files_and_folders() {
    let dir = tempfile::tempdir().expect("temp dir");
    let f = dir.path().join("f");
    std::fs::write(&f, b"x").expect("write");
    sync_path(&f).expect("file synced");
    sync_dir(dir.path());
    assert!(sync_path(&dir.path().join("missing")).is_err());
}
