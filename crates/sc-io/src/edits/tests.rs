//! Unit tests of the private parts of `crates/sc-io/src/edits.rs`.
use super::*;

fn saved(path: &str) -> SavedEdit {
    SavedEdit {
        schema: EDIT_SCHEMA,
        path: path.into(),
        audio: AudioIdentity {
            frames: 44_100 * 200,
            sample_rate: 44_100,
            integrated: Some(Lufs(-9.25)),
        },
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
    assert!(store.get("/music/a.flac").is_none());
    store.put(&saved("/music/a.flac")).unwrap();
    assert_eq!(store.get("/music/a.flac"), Some(saved("/music/a.flac")));
    assert!(store.get("/music/b.flac").is_none(), "other file");
    store.remove("/music/a.flac").unwrap();
    store.remove("/music/a.flac").unwrap();
    assert!(store.get("/music/a.flac").is_none());
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
    assert_eq!(store.get("/music/a.flac"), Some(newer));
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
    std::fs::write(store.entry_path("/music/a.flac"), b"{\"schema\": 1, \"pa").unwrap();
    assert!(store.get("/music/a.flac").is_none());
    let other = SavedEdit {
        schema: EDIT_SCHEMA + 1,
        ..saved("/music/a.flac")
    };
    store.put(&other).unwrap();
    assert!(store.get("/music/a.flac").is_none(), "other schema");
}

#[test]
fn a_saved_edit_is_readable_json() {
    let json = serde_json::to_string(&saved("/music/a.flac")).unwrap();
    assert!(json.contains("\"bpmRange\":[70.0,180.0]"), "{json}");
    assert!(json.contains("\"integrated\":-9.25"), "{json}");
    assert!(json.contains("\"octave\":-1"), "{json}");
}
