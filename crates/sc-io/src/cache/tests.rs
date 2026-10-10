//! Unit tests of the private parts of `crates/sc-io/src/cache.rs`.
use super::*;

#[test]
fn nfc_normalises_decomposed_names() {
    // "ü" as u + combining diaeresis (NFD, what macOS stores) becomes one code point.
    let nfd = Path::new("/music/u\u{0308}ber.flac");
    assert_eq!(nfc(nfd), "/music/\u{00fc}ber.flac");
    assert_eq!(
        blake3::hash(nfc(nfd).as_bytes()),
        blake3::hash("/music/\u{00fc}ber.flac".as_bytes())
    );
}

#[test]
fn a_schema_1_record_is_served_without_its_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path());
    let key = CacheKey {
        size: 1,
        mtime_ns: 2,
        ino: 5,
        ctime_ns: 6,
        settings_hash: 3,
        version: "0.0.3".into(),
    };
    let entry = serde_json::json!({
        "schema": CACHE_SCHEMA,
        "key": key,
        "record": {
            "schema": 1, "version": "0.0.3", "path": "/a.wav", "size": 1, "mtimeNs": 2,
            "spec": {"sampleRate": 44100, "channels": 2}, "frames": 1, "duration": 1.0,
            "delay": 0, "padding": 0,
            "loudness": {"integrated": null, "momentaryMax": null, "shortTermMax": null,
                "shortTermP95": null, "shortTermTop30": null, "lra": null, "truePeak": -1.0,
                "samplePeak": -1.0, "plr": null, "dualMono": false,
                "timeline": {"hopMs": 100, "shortTerm": []}},
            "grid": null, "gridSkipped": null,
            "tags": {"bpm": null, "genre": null, "title": null, "artist": null},
            "evidence": {"beats": [22050], "downbeats": [], "downbeatLogits50fps": [0.5],
                "kickOnsets": [22050]}
        }
    });
    std::fs::write(cache.entry_path("/a.wav"), entry.to_string()).unwrap();
    let record = cache.get("/a.wav", &key).expect("served");
    assert_eq!(record.schema, 1);
    assert_eq!(record.evidence, None);

    // Any other unreadable record is a miss.
    let mut broken = entry;
    broken["record"]["schema"] = 2.into();
    std::fs::write(cache.entry_path("/a.wav"), broken.to_string()).unwrap();
    assert!(cache.get("/a.wav", &key).is_none());
}

#[test]
fn entry_names_are_blake3_of_the_path() {
    let cache = Cache::open("/tmp/sc-cache-test");
    let name = cache.entry_path("/music/a.flac");
    assert_eq!(name.extension().unwrap(), "json");
    assert_eq!(name.file_stem().unwrap().len(), 64);
    assert_ne!(name, cache.entry_path("/music/b.flac"));
}

#[test]
fn an_entry_of_the_first_wrapper_schema_misses() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path());
    let entry = serde_json::json!({
        "schema": 1,
        "key": {"size": 1, "mtimeNs": 2, "settingsHash": 3, "version": "0.0.3"},
        "record": {}
    });
    std::fs::write(cache.entry_path("/a.wav"), entry.to_string()).unwrap();
    let key = CacheKey {
        size: 1,
        mtime_ns: 2,
        ino: 0,
        ctime_ns: 0,
        settings_hash: 3,
        version: "0.0.3".into(),
    };
    assert!(cache.get("/a.wav", &key).is_none());
}

#[test]
fn a_file_replaced_at_the_same_length_and_time_gets_another_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.wav");
    std::fs::write(&path, b"one").unwrap();
    let settings = sc_core::analysis::AnalysisSettings::default();
    let (_, before) = Cache::key_for(&path, &settings).unwrap();
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    // Replaced by a rename, as a write transaction does, with the modification time put back.
    let temp = dir.path().join(".a.tmp");
    std::fs::write(&temp, b"two").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&temp)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    std::fs::rename(&temp, &path).unwrap();
    let (_, after) = Cache::key_for(&path, &settings).unwrap();
    assert_eq!((after.size, after.mtime_ns), (before.size, before.mtime_ns));
    assert_ne!(after, before);
}

#[test]
fn an_entry_is_removed_by_its_file_path() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path().join("cache"));
    let file = dir.path().join("a.wav");
    std::fs::create_dir_all(cache.dir()).unwrap();
    std::fs::write(cache.entry_path(&nfc(&file)), b"{}").unwrap();
    assert!(cache.remove(&file).unwrap());
    assert!(!cache.entry_path(&nfc(&file)).exists());
    assert!(!cache.remove(&file).unwrap(), "nothing left");
}
