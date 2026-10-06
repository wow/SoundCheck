//! Unit tests of `cuesheet.rs`: the literal positions of a two-track sheet after a 441-sample
//! trim, clamping, the CD-DA refusal, malformed sheets.

use super::*;

/// (offset, number, [(index offset, index number)]).
type Track = (u64, u8, Vec<(u64, u8)>);

fn sheet(cdda: bool, tracks: &[Track]) -> Vec<u8> {
    let mut p = vec![0_u8; 128];
    p.extend_from_slice(&0_u64.to_be_bytes());
    p.push(if cdda { 0x80 } else { 0 });
    p.extend(std::iter::repeat_n(0_u8, 258));
    p.push(u8::try_from(tracks.len()).expect("few"));
    for (offset, number, indices) in tracks {
        p.extend_from_slice(&offset.to_be_bytes());
        p.push(*number);
        p.extend_from_slice(&[0x11; 12]); // ISRC bytes, kept
        p.extend_from_slice(&[0; 14]);
        p.push(u8::try_from(indices.len()).expect("few"));
        for (o, n) in indices {
            p.extend_from_slice(&o.to_be_bytes());
            p.push(*n);
            p.extend_from_slice(&[0; 3]);
        }
    }
    p
}

#[test]
fn a_441_sample_trim_shrinks_the_pregap_and_moves_track_2() {
    let mut p = sheet(
        false,
        &[
            (0, 1, vec![(0, 0), (588, 1)]),
            (2205, 2, vec![(0, 0), (588, 1)]),
            (20_000, 255, vec![]),
        ],
    );
    let want = sheet(
        false,
        &[
            (0, 1, vec![(0, 0), (147, 1)]),
            (1764, 2, vec![(0, 0), (588, 1)]),
            (19_559, 255, vec![]),
        ],
    );
    assert_eq!(shift_cuesheet(&mut p, 441, 19_559), Ok(true));
    assert_eq!(p, want);
    // No trim and the same total: nothing changes.
    assert_eq!(shift_cuesheet(&mut p, 0, 19_559), Ok(false));
    assert_eq!(p, want);
}

#[test]
fn a_track_inside_the_trim_clamps_to_zero() {
    let mut p = sheet(
        false,
        &[
            (100, 1, vec![(0, 1)]),
            (300, 2, vec![]),
            (1000, 255, vec![]),
        ],
    );
    shift_cuesheet(&mut p, 500, 500).expect("shifts");
    let want = sheet(
        false,
        &[(0, 1, vec![(0, 1)]), (0, 2, vec![]), (500, 255, vec![])],
    );
    assert_eq!(p, want);
}

#[test]
fn cd_da_needs_whole_sectors() {
    let tracks = [(0, 1, vec![(0, 1)]), (44_100, 170, vec![])];
    let mut p = sheet(true, &tracks);
    let before = p.clone();
    assert_eq!(
        shift_cuesheet(&mut p, 441, 43_659),
        Err(CuesheetError::CdDaAlignment)
    );
    assert_eq!(p, before);
    assert_eq!(shift_cuesheet(&mut p, 588, 43_512), Ok(true));
    assert_eq!(
        p,
        sheet(true, &[(0, 1, vec![(0, 1)]), (43_512, 170, vec![])])
    );
}

#[test]
fn malformed_sheets_are_refused_unchanged() {
    let mut short = vec![0_u8; 100];
    assert!(matches!(
        shift_cuesheet(&mut short, 1, 1),
        Err(CuesheetError::Malformed(_))
    ));
    let mut cut = sheet(false, &[(0, 1, vec![(0, 1), (10, 2)])]);
    cut.truncate(cut.len() - 5);
    let before = cut.clone();
    assert!(matches!(
        shift_cuesheet(&mut cut, 1, 1),
        Err(CuesheetError::Malformed(_))
    ));
    assert_eq!(cut, before);
    // Index 01 before index 00 after clamping: out of order.
    let mut disorder = sheet(false, &[(1000, 1, vec![(500, 0), (0, 1)])]);
    assert!(matches!(
        shift_cuesheet(&mut disorder, 0, 1),
        Err(CuesheetError::Malformed(_))
    ));
}
