//! Unit tests of the private parts of `crates/sc-analysis/src/grid/octave.rs`.
use super::*;

#[test]
fn snap_only_within_the_fit_noise() {
    assert!((snap_bpm(127.98, 0.001) - 127.98).abs() < 1e-12);
    assert!((snap_bpm(128.0015, 0.0001) - 128.0).abs() < 1e-12);
    assert!((snap_bpm(128.004, 0.002) - 128.0).abs() < 1e-12);
    assert!((snap_bpm(128.01, 0.002) - 128.01).abs() < 1e-12);
}

#[test]
fn genre_keywords() {
    assert_eq!(genre_range(Some("Drum & Bass")), Some((160.0, 180.0)));
    assert_eq!(genre_range(Some("Progressive House")), Some((120.0, 130.0)));
    assert_eq!(genre_range(Some("Hip-Hop/Rap")), Some((80.0, 100.0)));
    assert_eq!(genre_range(Some("Türkçe Pop")), None);
    assert_eq!(genre_range(None), None);
}
