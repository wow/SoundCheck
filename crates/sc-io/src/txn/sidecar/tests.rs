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
        backup_target: None,
        volume: None,
        folder: None,
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
                trim_frames: Some(0),
                trim_requested_frames: Some(0),
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

/// The `render` object of a sidecar written before the cut was recorded (a 13,009-frame cut).
const OLD_RENDER: &str = r#"{
  "frames_in": 1000000,
  "frames_out": 986991,
  "sample_rate_hz": 44100,
  "channels": 2,
  "bits_out": 24,
  "exact": false,
  "dithered": false,
  "samples_saturated": 0,
  "pcm_blake3": "cc",
  "blocks": { "carried": 3, "patched": 1, "edited": 1, "replaced": 2, "dropped": 0 },
  "tags_added": true,
  "tags_not_added": null,
  "stale_loudness_tags": []
}"#;

#[test]
fn an_old_record_without_the_cut_derives_it_from_the_frame_counts() {
    let old: RenderSummary = serde_json::from_str(OLD_RENDER).expect("an old record reads");
    assert_eq!((old.trim_frames, old.trim_requested_frames), (None, None));
    assert_eq!(old.trim_frames(), 13_009);
    assert_eq!(old.trim_requested_frames(), 13_009);
    // The same in a journal record, with a request from before snapped cuts were flagged.
    let record = format!(
        r#"{{ "request": {{ "gain_db": -2.0, "trim_frames": 13009, "bits": null,
              "loudness": null, "tag_edits": [] }},
             "render": {OLD_RENDER} }}"#
    );
    let record: Record = serde_json::from_str(&record).expect("an old journal record reads");
    assert_eq!(record.request.trim_snapped_from, None);
    assert_eq!(record.render.trim_frames(), 13_009);
    // A new record keeps both values as made, also when they differ, and writes them out.
    let mut new = old.clone();
    new.trim_frames = Some(12_970);
    new.trim_requested_frames = Some(13_009);
    assert_eq!(
        (new.trim_frames(), new.trim_requested_frames()),
        (12_970, 13_009)
    );
    let text = serde_json::to_string(&new).expect("serialises");
    assert!(
        text.contains("\"trim_frames\":12970,\"trim_requested_frames\":13009"),
        "{text}"
    );
    // A request that is not flagged as snapped writes no flag, so its text is unchanged.
    let request = serde_json::to_string(&record.request).expect("serialises");
    assert!(!request.contains("trim_snapped_from"), "{request}");
}
