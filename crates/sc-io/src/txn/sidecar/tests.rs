//! Unit tests of `sidecar.rs`: the path, the fixed key order, determinism and the atomic
//! replace.

use super::*;
use crate::txn::journal::State;

fn entry(path: &Path) -> Entry {
    Entry {
        txn: "t-1".into(),
        kind: TxnKind::InPlace,
        state: State::MetadataDone,
        reached: State::MetadataDone,
        started_at: "2026-10-07T10:00:00Z".into(),
        path: path.to_path_buf(),
        source: path.to_path_buf(),
        temp: path.with_file_name(".x.tmp"),
        backup_temp: None,
        backup: Some(PathBuf::from("/backups/2026-10-07/Disk/a.wav")),
        keep_mtime: true,
        sidecar: true,
        undoes: None,
        original_blake3: Some("aa".into()),
        original_bytes: Some(10),
        output_blake3: Some("bb".into()),
        output_bytes: Some(12),
        record: Some(Record {
            request: RenderRequest {
                gain_db: -3.0,
                ..RenderRequest::default()
            },
            render: RenderSummary {
                frames_in: 100,
                frames_out: 100,
                sample_rate_hz: 44_100,
                channels: 2,
                bits_out: 16,
                exact: false,
                dithered: true,
                samples_saturated: 0,
                pcm_blake3: "cc".into(),
                blocks: BlockCounts {
                    carried: 1,
                    patched: 0,
                    edited: 0,
                    replaced: 2,
                    dropped: 0,
                },
                tags_added: false,
                tags_not_added: None,
                stale_loudness_tags: Vec::new(),
            },
        }),
        outcome: None,
        error: None,
        notes: Vec::new(),
        undone: false,
    }
}

#[test]
fn the_sidecar_sits_next_to_the_file() {
    assert_eq!(
        sidecar_path(Path::new("/m/a.wav")),
        PathBuf::from("/m/a.wav.soundcheck.json")
    );
}

#[test]
fn the_text_has_a_fixed_key_order_and_is_deterministic() {
    let e = entry(Path::new("/m/a.wav"));
    let text = render_text(&e, &[]).expect("text");
    assert_eq!(text, render_text(&e, &[]).expect("again"));
    assert!(text.ends_with("}\n"));
    let keys = [
        "\"schema\": 1",
        "\"app\": \"SoundCheck\"",
        "\"version\"",
        "\"file\": \"a.wav\"",
        "\"transaction\": \"t-1\"",
        "\"processed_at\"",
        "\"mode\": \"in_place\"",
        "\"original\"",
        "\"output\"",
        "\"request\"",
        "\"render\"",
        "\"backup\"",
        "\"metadata\"",
    ];
    let at: Vec<usize> = keys
        .iter()
        .map(|k| text.find(k).unwrap_or_else(|| panic!("{k} missing")))
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{text}");
    let mut no_record = e.clone();
    no_record.record = None;
    assert!(render_text(&no_record, &[]).is_err());
}

#[test]
fn writing_replaces_an_older_sidecar_and_leaves_no_temp() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("a.wav");
    std::fs::write(sidecar_path(&file), b"old").expect("old sidecar");
    let path = write(&entry(&file), &["a note".into()]).expect("written");
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("JSON");
    assert_eq!(doc["metadata"]["notes"][0], "a note");
    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 1);
    assert!(remove(&file).expect("removed"));
    assert!(!remove(&file).expect("nothing left"));
}
