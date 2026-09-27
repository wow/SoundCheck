//! Unit tests of the private parts of `crates/sc-engine/src/expand.rs`.
use super::*;

#[test]
fn natural_order_compares_numbers_by_value_and_ignores_case() {
    let mut names = vec![
        "Track 10.wav",
        "track 2.wav",
        "Track 1.wav",
        "B",
        "a",
        "Track 02b",
    ];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        names,
        vec![
            "a",
            "B",
            "Track 1.wav",
            "track 2.wav",
            "Track 02b",
            "Track 10.wav"
        ]
    );
}

#[test]
fn only_single_dot_names_are_hidden() {
    assert!(is_hidden(".Trash"));
    assert!(is_hidden("._Track 2.wav"));
    assert!(is_hidden(".DS_Store"));
    assert!(!is_hidden("...Baby One More Time (Digital Deluxe Version)"));
    assert!(!is_hidden("..Two Dots"));
    assert!(!is_hidden("Track.flac"));
}

#[test]
fn audio_extensions_ignore_case() {
    assert!(is_audio(Path::new("x/Y.FLAC")));
    assert!(is_audio(Path::new("x.Aiff")));
    assert!(!is_audio(Path::new("notes.txt")));
    assert!(!is_audio(Path::new("noext")));
}
