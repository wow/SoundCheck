//! Dropped paths to rows: the walk order, what is skipped, duplicates, and the parallel probe.

mod common;

use std::path::PathBuf;

use sc_core::plan::Codec;
use sc_engine::{collect_audio_files, probe_all};

#[test]
fn folders_are_walked_in_natural_order_without_hidden_files_links_or_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("crate");
    std::fs::create_dir_all(root.join("B")).unwrap();
    std::fs::create_dir_all(root.join(".Trash")).unwrap();
    std::fs::create_dir_all(root.join("...Baby One More Time")).unwrap();
    common::tone_wav(
        &root.join("...Baby One More Time"),
        "01 Title.wav",
        0.5,
        0.1,
    );
    common::tone_wav(&root, "Track 10.wav", 0.5, 0.1);
    common::tone_wav(&root, "Track 2.WAV", 0.5, 0.1);
    common::tone_wav(&root, ".hidden.wav", 0.5, 0.1);
    common::tone_wav(&root, "._Track 2.WAV", 0.5, 0.1);
    common::tone_wav(&root.join(".Trash"), "gone.wav", 0.5, 0.1);
    common::tone_wav(&root.join("B"), "inner.wav", 0.5, 0.1);
    std::fs::write(root.join("notes.txt"), b"not audio").unwrap();
    std::os::unix::fs::symlink(root.join("B"), root.join("link")).unwrap();

    let dropped = vec![
        root.clone(),
        root.join("Track 10.wav"),
        root.join("notes.txt"),
    ];
    let files = collect_audio_files(&dropped);
    let names: Vec<String> = files
        .iter()
        .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "...Baby One More Time/01 Title.wav",
            "B/inner.wav",
            "Track 2.WAV",
            "Track 10.wav"
        ]
    );
}

#[test]
fn a_symlinked_file_dropped_directly_is_followed() {
    let dir = tempfile::tempdir().unwrap();
    let real = common::tone_wav(dir.path(), "real.wav", 0.5, 0.1);
    let link = dir.path().join("alias.wav");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(collect_audio_files(std::slice::from_ref(&link)), vec![link]);
}

#[test]
fn probes_come_back_in_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = (0..9)
        .map(|i| common::tone_wav(dir.path(), &format!("{i}.wav"), 0.1 * f64::from(i + 1), 0.1))
        .chain([common::corrupt_wav(dir.path(), "zz.wav")])
        .collect();
    let infos = probe_all(&files, 4);
    assert_eq!(infos.len(), 10);
    for (i, info) in infos.iter().take(9).enumerate() {
        assert_eq!(info.codec, Codec::Wav);
        let expected = 0.1 * f64::from(u32::try_from(i).unwrap() + 1);
        assert!(
            (info.duration.unwrap().0 - expected).abs() < 0.01,
            "{i}: {:?}",
            info.duration
        );
    }
    assert_eq!(infos[9].codec, Codec::Wav, "named .wav");
    assert_eq!(infos[9].sample_rate, None, "unreadable");
}
