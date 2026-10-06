//! Unit tests of `seektable.rs`: shape counting, point placement, more points than frames.

use super::*;

fn point(sample: u64, offset: u64, samples: u16) -> Vec<u8> {
    let mut p = sample.to_be_bytes().to_vec();
    p.extend_from_slice(&offset.to_be_bytes());
    p.extend_from_slice(&samples.to_be_bytes());
    p
}

fn placeholder() -> Vec<u8> {
    let mut p = PLACEHOLDER.to_be_bytes().to_vec();
    p.extend_from_slice(&[0; 10]);
    p
}

#[test]
fn shape_counts_real_and_placeholder_points() {
    let p = [point(0, 0, 4096), point(8192, 900, 4096), placeholder()].concat();
    assert_eq!(
        SeekShape::of(&p),
        Ok(SeekShape {
            real: 2,
            placeholders: 1
        })
    );
    assert_eq!(SeekShape::of(&p).expect("ok").bytes(), p.len());
    assert!(SeekShape::of(&p[..17]).is_err());
    assert_eq!(
        SeekShape::of(&[]),
        Ok(SeekShape {
            real: 0,
            placeholders: 0
        })
    );
}

#[test]
fn points_land_on_evenly_spaced_frames() {
    // Five frames of 4096 samples, the last one 3175: 19,559 samples.
    let offsets = [0, 100, 250, 380, 500];
    let shape = SeekShape {
        real: 3,
        placeholders: 2,
    };
    let got = build(shape, &offsets, 4096, 19_559);
    // k * 5 / 3 for k = 0, 1, 2: frames 0, 1, 3.
    let want = [
        point(0, 0, 4096),
        point(4096, 100, 4096),
        point(12_288, 380, 4096),
        placeholder(),
        placeholder(),
    ]
    .concat();
    assert_eq!(got, want);
    let all = build(
        SeekShape {
            real: 5,
            placeholders: 0,
        },
        &offsets,
        4096,
        19_559,
    );
    assert_eq!(&all[4 * 18..], &point(16_384, 500, 3175)[..]);
}

#[test]
fn more_points_than_frames_become_placeholders_and_keep_the_size() {
    let shape = SeekShape {
        real: 4,
        placeholders: 1,
    };
    let got = build(shape, &[0, 70], 4096, 5000);
    let want = [
        point(0, 0, 4096),
        point(4096, 70, 904),
        placeholder(),
        placeholder(),
        placeholder(),
    ]
    .concat();
    assert_eq!(got, want);
    assert_eq!(got.len(), shape.bytes());
}
