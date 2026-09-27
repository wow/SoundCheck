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
