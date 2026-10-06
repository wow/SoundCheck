//! Unit tests of `crates/sc-io/src/render/patch.rs` on hand-built payloads.
use super::*;

fn cue(points: &[u32]) -> Vec<u8> {
    let mut p = u32::try_from(points.len())
        .expect("few")
        .to_le_bytes()
        .to_vec();
    for (i, pos) in points.iter().enumerate() {
        p.extend_from_slice(&u32::try_from(i).expect("few").to_le_bytes());
        p.extend_from_slice(&pos.to_le_bytes());
        p.extend_from_slice(b"data");
        p.extend_from_slice(&[0; 8]);
        p.extend_from_slice(&pos.to_le_bytes());
    }
    p
}

fn le32(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(p[at..at + 4].try_into().expect("4 bytes"))
}

#[test]
fn cue_positions_shift_and_clamp() {
    let mut p = cue(&[0, 300, 11_025]);
    let before = p.clone();
    shift_cue(&mut p, 441).expect("valid");
    let got: Vec<_> = (0..3)
        .map(|i| (le32(&p, 8 + 24 * i), le32(&p, 24 + 24 * i)))
        .collect();
    assert_eq!(got, [(0, 0), (0, 0), (10_584, 10_584)]);
    // Only the two position fields changed.
    for (i, (a, b)) in before.iter().zip(&p).enumerate() {
        let k = (i.saturating_sub(4)) % 24;
        if i < 4 || !(4..8).contains(&k) && !(20..24).contains(&k) {
            assert_eq!(a, b, "byte {i}");
        }
    }
    let mut short = cue(&[1, 2]);
    short.truncate(30);
    assert!(matches!(
        shift_cue(&mut short, 1),
        Err(PatchError::Malformed(_))
    ));
}

fn smpl(loops: &[(u32, u32)]) -> Vec<u8> {
    let mut p = vec![0; 28];
    p.extend_from_slice(&u32::try_from(loops.len()).expect("few").to_le_bytes());
    p.extend_from_slice(&0_u32.to_le_bytes());
    for (start, end) in loops {
        p.extend_from_slice(&[0; 8]);
        p.extend_from_slice(&start.to_le_bytes());
        p.extend_from_slice(&end.to_le_bytes());
        p.extend_from_slice(&[0; 8]);
    }
    p
}

#[test]
fn smpl_loops_after_the_trim_shift() {
    let mut p = smpl(&[(441, 11_025), (500, 900)]);
    shift_smpl(&mut p, 441).expect("both loops start at or after the trim");
    assert_eq!((le32(&p, 44), le32(&p, 48)), (0, 10_584));
    assert_eq!((le32(&p, 68), le32(&p, 72)), (59, 459));
}

#[test]
fn smpl_loops_touching_the_cut_are_refused() {
    // Wholly inside the cut, ending exactly at it, and only starting inside it (the loop would
    // lose 341 samples of its start): all refused, the payload untouched.
    for (start, end) in [
        (100, 400),
        (100, 441),
        (100, 10_000),
        (0, 11_025),
        (440, 442),
    ] {
        let mut p = smpl(&[(1_000, 2_000), (start, end)]);
        let untouched = p.clone();
        assert_eq!(
            shift_smpl(&mut p, 441),
            Err(PatchError::LoopInTrim { start, end }),
            "{start}..{end}"
        );
        assert_eq!(p, untouched, "a refused patch leaves the payload alone");
    }
    let mut p = smpl(&[(100, 400)]);
    let untouched = p.clone();
    shift_smpl(&mut p, 0).expect("no trim, nothing to refuse");
    assert_eq!(p, untouched);
}

#[test]
fn mark_positions_shift_over_padded_names() {
    let mut p = 3_u16.to_be_bytes().to_vec();
    for (id, pos, name) in [(1_i16, 0_u32, "Start"), (2, 300, "In"), (3, 12_000, "Drop")] {
        p.extend_from_slice(&id.to_be_bytes());
        p.extend_from_slice(&pos.to_be_bytes());
        p.push(u8::try_from(name.len()).expect("short"));
        p.extend_from_slice(name.as_bytes());
        if (1 + name.len()) % 2 == 1 {
            p.push(0);
        }
    }
    shift_mark(&mut p, 441).expect("valid");
    let be = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    // Markers start at 2, 2 + 6 + 6 = 14, 14 + 6 + 4 = 24.
    assert_eq!((be(4), be(16), be(26)), (0, 0, 11_559));
    let mut short = p[..20].to_vec();
    assert!(shift_mark(&mut short, 1).is_err());
}

#[test]
fn fact_takes_the_new_length() {
    let mut p = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    set_fact(&mut p, 19_559).expect("valid");
    assert_eq!(p, [0x67, 0x4C, 0, 0, 0xEE]);
    assert!(set_fact(&mut [0; 3], 1).is_err());
    assert!(set_fact(&mut [0; 4], 1 << 32).is_err());
}

fn bext(version: u16) -> Vec<u8> {
    let mut p = vec![0x11; 610];
    p[BEXT_TIME_REFERENCE..BEXT_TIME_REFERENCE + 8].copy_from_slice(&158_760_000_u64.to_le_bytes());
    p[BEXT_VERSION..BEXT_VERSION + 2].copy_from_slice(&version.to_le_bytes());
    p
}

#[test]
fn bext_time_reference_version_and_loudness() {
    let mut p = bext(1);
    let before = p.clone();
    assert_eq!(patch_bext(&mut p, 441, BextUpdate::Keep), Ok(true));
    let t = u64::from_le_bytes(p[338..346].try_into().expect("8 bytes"));
    assert_eq!(t, 158_760_441);
    assert_eq!(
        p[346..],
        before[346..],
        "no loudness: version and fields untouched"
    );
    let fields = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    assert_eq!(patch_bext(&mut p, 0, BextUpdate::Set(fields)), Ok(true));
    assert_eq!(p[346..348], [2, 0], "version 1 upgraded");
    assert_eq!(p[412..422], fields);
    assert_eq!(p[348..412], before[348..412]);
    assert_eq!(p[422..], before[422..]);
}

#[test]
fn bext_gain_without_loudness_clears_only_version_2() {
    let mut v2 = bext(2);
    assert_eq!(
        patch_bext(&mut v2, 0, BextUpdate::ClearIfVersion2),
        Ok(true)
    );
    assert_eq!(v2[412..422], [0xFF, 0x7F].repeat(5)[..]);
    for version in [0, 1] {
        let mut p = bext(version);
        let before = p.clone();
        assert_eq!(
            patch_bext(&mut p, 0, BextUpdate::ClearIfVersion2),
            Ok(false)
        );
        assert_eq!(p, before, "version {version} has no loudness to clear");
        // With a trim the time reference still moves, the reserved bytes stay.
        assert_eq!(
            patch_bext(&mut p, 441, BextUpdate::ClearIfVersion2),
            Ok(true)
        );
        assert_eq!(p[346..], before[346..], "version {version}");
    }
}

#[test]
fn a_short_bext_is_only_refused_when_it_must_change() {
    for len in [0, 100, 347, 601] {
        // Version 0 where the field exists: nothing to clear.
        let mut p = vec![0; len];
        let before = p.clone();
        assert_eq!(patch_bext(&mut p, 0, BextUpdate::Keep), Ok(false), "{len}");
        assert_eq!(
            patch_bext(&mut p, 0, BextUpdate::ClearIfVersion2),
            Ok(false),
            "{len}"
        );
        assert_eq!(p, before);
        assert!(patch_bext(&mut p, 1, BextUpdate::Keep).is_err(), "{len}");
        assert!(
            patch_bext(&mut p, 0, BextUpdate::Set([0; 10])).is_err(),
            "{len}"
        );
    }
    // A short chunk claiming version 2 still cannot take the fields.
    let mut p = vec![0; 400];
    p[BEXT_VERSION] = 2;
    assert!(patch_bext(&mut p, 0, BextUpdate::ClearIfVersion2).is_err());
}
