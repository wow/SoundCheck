//! Unit tests of the private parts of `crates/sc-io/src/edits.rs`.
use super::*;

fn audio() -> AudioIdentity {
    AudioIdentity {
        frames: 44_100 * 200,
        sample_rate: 44_100,
        integrated: Some(Lufs(-9.25)),
    }
}

fn saved(path: &str) -> SavedEdit {
    SavedEdit {
        schema: EDIT_SCHEMA,
        path: path.into(),
        audio: audio(),
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        edit: GridEdit {
            octave: -1,
            anchor: Some(SampleIndex(22_050)),
            ..GridEdit::default()
        },
        grid: Some(GridPin {
            bpm: Bpm(62.0),
            anchor: SampleIndex(22_050),
            meter: Meter::four_four(),
        }),
        confirmed: true,
    }
}

#[test]
fn an_edit_round_trips_and_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let store = EditStore::open(dir.path().join("edits"));
    assert!(store.get("/music/a.flac", &audio()).is_none());
    store.put(&saved("/music/a.flac")).unwrap();
    assert_eq!(
        store.get("/music/a.flac", &audio()),
        Some(saved("/music/a.flac"))
    );
    assert!(store.get("/music/b.flac", &audio()).is_none(), "other file");
    store.remove("/music/a.flac", &audio()).unwrap();
    store.remove("/music/a.flac", &audio()).unwrap();
    assert!(store.get("/music/a.flac", &audio()).is_none());
}

#[test]
fn a_newer_save_replaces_the_edit_and_no_temp_file_stays() {
    let dir = tempfile::tempdir().unwrap();
    let store = EditStore::open(dir.path());
    store.put(&saved("/music/a.flac")).unwrap();
    let newer = SavedEdit {
        confirmed: false,
        ..saved("/music/a.flac")
    };
    store.put(&newer).unwrap();
    assert_eq!(store.get("/music/a.flac", &audio()), Some(newer));
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

#[test]
fn a_damaged_or_foreign_file_is_no_edit() {
    let dir = tempfile::tempdir().unwrap();
    let store = EditStore::open(dir.path());
    std::fs::write(
        store.entry_path("/music/a.flac", &audio()),
        b"{\"schema\": 1, \"pa",
    )
    .unwrap();
    assert!(store.get("/music/a.flac", &audio()).is_none());
    let other = SavedEdit {
        schema: EDIT_SCHEMA + 1,
        ..saved("/music/a.flac")
    };
    store.put(&other).unwrap();
    assert!(
        store.get("/music/a.flac", &audio()).is_none(),
        "other schema"
    );
}

#[test]
fn a_saved_edit_is_readable_json() {
    let json = serde_json::to_string(&saved("/music/a.flac")).unwrap();
    assert!(json.contains("\"bpmRange\":[70.0,180.0]"), "{json}");
    assert!(json.contains("\"integrated\":-9.25"), "{json}");
    assert!(json.contains("\"octave\":-1"), "{json}");
}

#[test]
fn a_path_keeps_one_edit_per_audio() {
    let dir = tempfile::tempdir().unwrap();
    let store = EditStore::open(dir.path());
    let original = saved("/music/a.flac");
    // The same file exported: shorter by a cut, at another loudness.
    let exported = SavedEdit {
        audio: AudioIdentity {
            frames: original.audio.frames - 13_009,
            integrated: Some(Lufs(-11.0)),
            ..original.audio
        },
        confirmed: false,
        ..original.clone()
    };
    store.put(&original).unwrap();
    store.put(&exported).unwrap();
    assert_eq!(store.get("/music/a.flac", &audio()), Some(original.clone()));
    assert_eq!(
        store.get("/music/a.flac", &exported.audio),
        Some(exported.clone())
    );
    store.remove("/music/a.flac", &exported.audio).unwrap();
    assert_eq!(store.get("/music/a.flac", &audio()), Some(original));
    assert!(store.get("/music/a.flac", &exported.audio).is_none());
}

#[test]
fn an_edit_named_by_its_path_alone_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let store = EditStore::open(dir.path());
    let old = saved("/music/a.flac");
    let legacy = store.legacy_entry_path("/music/a.flac");
    std::fs::write(&legacy, serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(store.get("/music/a.flac", &audio()), Some(old.clone()));
    let other = AudioIdentity {
        frames: 1,
        ..audio()
    };
    assert!(store.get("/music/a.flac", &other).is_none(), "other audio");
    // A save of other audio leaves it; a save of its audio replaces it.
    store
        .put(&SavedEdit {
            audio: other,
            ..old.clone()
        })
        .unwrap();
    assert!(legacy.exists());
    let newer = SavedEdit {
        confirmed: false,
        ..old
    };
    store.put(&newer).unwrap();
    assert!(!legacy.exists());
    assert_eq!(store.get("/music/a.flac", &audio()), Some(newer));
}
