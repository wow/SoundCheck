//! Unit tests of `crates/sc-core/src/render.rs`.
use super::*;

#[test]
fn bext_loudness_bytes_are_little_endian_hundredths() {
    let l = BextLoudness::from_measurements(
        Some(Lufs(-11.23)),
        None,
        Some(DbTp(-0.52)),
        Some(Lufs(-8.12)),
        Some(Lufs(-9.05)),
    );
    assert_eq!(
        l.to_le_bytes(),
        [0x9D, 0xFB, 0xFF, 0x7F, 0xCC, 0xFF, 0xD4, 0xFC, 0x77, 0xFC]
    );
    assert_eq!(
        BextLoudness::ALL_UNMEASURED.to_le_bytes(),
        [0xFF, 0x7F].repeat(5)[..]
    );
}

#[test]
fn values_that_do_not_fit_are_unmeasured() {
    let l = BextLoudness::from_measurements(
        Some(Lufs(f64::NEG_INFINITY)),
        Some(Lu(400.0)),
        Some(DbTp(f64::NAN)),
        Some(Lufs(-327.0)),
        Some(Lufs(327.67)),
    );
    assert_eq!(l.integrated_lufs_x100, BextLoudness::UNMEASURED);
    assert_eq!(l.range_lu_x100, BextLoudness::UNMEASURED);
    assert_eq!(l.max_true_peak_dbtp_x100, BextLoudness::UNMEASURED);
    assert_eq!(l.max_momentary_lufs_x100, -32_700);
    assert_eq!(l.max_short_term_lufs_x100, BextLoudness::UNMEASURED);
}

#[test]
fn default_request_is_the_identity() {
    let r = RenderRequest::default();
    assert!(r.gain_db == 0.0 && r.trim_frames == 0 && r.bits.is_none());
    assert!(r.loudness.is_none() && r.tag_edits.is_empty());
}
