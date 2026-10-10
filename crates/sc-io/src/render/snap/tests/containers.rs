//! The frame the snap picks, on every container the renderer reads (WAV 16/24, 32-bit float
//! WAV, RF64, AIFF, AIFF-C, FLAC 16/24): the quietest frame strictly inside the window, the
//! latest of equally quiet frames, never a quieter frame after the request, and the same
//! choice when the window crosses the 4,096-frame block boundary of the readers. In every case
//! a render of the snapped cut flagged as such is byte for byte the render of the request.

use super::*;
use crate::iff::test_build::fmt;

/// A loud stereo signal (left 9,000..14,000 on the 16-bit scale, right minus half of it) with
/// the frames `quiet` set to the given left values.
fn signal(quiet: &[(u64, i32)]) -> Vec<i32> {
    (0..FRAMES)
        .flat_map(|i| {
            let n = i32::try_from(i).expect("small");
            let mut x = 9_000 + (n * 37) % 5_000;
            if n % 2 == 1 {
                x = -x;
            }
            if let Some((_, q)) = quiet.iter().find(|(at, _)| *at == i as u64) {
                x = *q;
            }
            [x, -x / 2]
        })
        .collect()
}

/// `samples` (16-bit scale) in each container: name, bytes, extension, tag label.
fn containers(samples: &[i32]) -> Vec<(&'static str, Vec<u8>, &'static str, &'static str)> {
    let frames = u32::try_from(FRAMES).expect("small");
    let x24: Vec<i32> = samples.iter().map(|s| s * 256).collect();
    let float: Vec<u8> = samples
        .iter()
        .flat_map(|s| {
            // 16-bit values are exact in f32.
            #[allow(clippy::cast_precision_loss)]
            let f = *s as f32 / 32_768.0;
            f.to_le_bytes()
        })
        .collect();
    let wav16 = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, RATE, 16))
        .chunk(b"data", &le_bytes(samples, 2))
        .build();
    let wav24 = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, RATE, 24))
        .chunk(b"data", &le_bytes(&x24, 3))
        .build();
    let wav_float = Form::riff()
        .chunk(b"fmt ", &fmt(3, 2, RATE, 8, 32))
        .chunk(b"data", &float)
        .build();
    let aiff = Form::aiff()
        .chunk(b"COMM", &comm(2, frames, 24, RATE, None))
        .chunk(b"SSND", &ssnd(0, &be_bytes(&x24, 3)))
        .build();
    let aifc = Form::aifc()
        .chunk(b"FVER", &0xA280_5140_u32.to_be_bytes())
        .chunk(b"COMM", &comm(2, frames, 16, RATE, Some(b"NONE")))
        .chunk(b"SSND", &ssnd(0, &be_bytes(samples, 2)))
        .build();
    let flac = |data: &[i32], bits: u8| {
        let (frames, info) = encode(data, RATE, 2, bits);
        file(
            &[],
            &info,
            &[(4, vorbis("ref", &["TITLE=t"]))],
            &frames,
            &[],
        )
    };
    vec![
        ("WAV 16", wav16, "wav", "TXXX:SOUNDCHECK"),
        ("WAV 24", wav24, "wav", "TXXX:SOUNDCHECK"),
        ("WAV float", wav_float, "wav", "TXXX:SOUNDCHECK"),
        (
            "RF64",
            rf64(&le_bytes(samples, 2)),
            "wav",
            "TXXX:SOUNDCHECK",
        ),
        ("AIFF 24", aiff, "aif", "TXXX:SOUNDCHECK"),
        ("AIFF-C", aifc, "aifc", "TXXX:SOUNDCHECK"),
        ("FLAC 16", flac(samples, 16), "flac", "SOUNDCHECK"),
        ("FLAC 24", flac(&x24, 24), "flac", "SOUNDCHECK"),
    ]
}

/// A 16-bit stereo RF64 file holding `data`: `ds64` with the RIFF and data sizes, the 32-bit
/// sizes of the container and `data` set to 0xFFFFFFFF (EBU Tech 3306).
fn rf64(data: &[u8]) -> Vec<u8> {
    let fmt = fmt_pcm(2, RATE, 16);
    let mut ds64 = Vec::new();
    let riff_size = (4 + 8 + 28 + 8 + fmt.len() + 8 + data.len()) as u64;
    ds64.extend_from_slice(&riff_size.to_le_bytes());
    ds64.extend_from_slice(&(data.len() as u64).to_le_bytes());
    ds64.extend_from_slice(&(FRAMES as u64).to_le_bytes());
    ds64.extend_from_slice(&0_u32.to_le_bytes());
    let mut out = b"RF64".to_vec();
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    for (id, size, payload) in [
        (b"ds64", 28, ds64.as_slice()),
        (b"fmt ", 16, fmt.as_slice()),
        (b"data", u32::MAX, data),
    ] {
        out.extend_from_slice(id);
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(payload);
    }
    out
}

#[test]
fn the_snap_picks_the_quietest_frame_ties_late_across_blocks_on_every_container() {
    // (requested cut, quiet frames, the snap). The window of a cut T is [T - 44, T].
    let cases: [(u64, Vec<(u64, i32)>, u64); 4] = [
        // Strictly inside: the quietest of two quiet frames; a silent frame after the request
        // is never taken.
        (5_000, vec![(4_970, 7), (4_983, 3), (5_003, 0)], 4_983),
        // A tie goes to the later frame.
        (5_000, vec![(4_970, 2), (4_990, -2)], 4_990),
        // The window [4,056, 4,100] crosses the readers' 4,096-frame block boundary: a tie
        // across it goes late, and a quieter frame before it wins.
        (4_100, vec![(4_060, 1), (4_098, -1)], 4_098),
        (4_100, vec![(4_090, 0), (4_099, 4)], 4_090),
    ];
    for (requested, quiet, expected) in cases {
        for (name, bytes, ext, label) in containers(&signal(&quiet)) {
            let at = format!("{name}, cut {requested}, quiet {quiet:?}");
            let src = Source::new(&bytes, ext);
            let snapped = snap_head_cut(&src.path, requested).expect(&at);
            assert_eq!(snapped, expected, "{at}");
            let (direct, direct_bytes) = src
                .render(&request(requested, None, label), "direct")
                .expect(&at);
            assert_eq!(direct.trim_frames, expected, "{at}: the render's snap");
            let (flagged, flagged_bytes) = src
                .render(&request(snapped, Some(requested), label), "flagged")
                .expect(&at);
            assert_eq!(flagged, direct, "{at}");
            assert!(flagged_bytes == direct_bytes, "{at}: not byte identical");
        }
    }
}
