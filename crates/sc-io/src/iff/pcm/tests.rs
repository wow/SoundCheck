use std::io::Cursor;
use std::path::Path;

use sc_core::Error;

use super::super::read_header;
use super::super::test_build::{Form, comm, fmt, fmt_extensible, fmt_pcm, ssnd};
use super::*;

const PATH: &str = "t";

fn ints(bytes: &[u8]) -> Vec<i32> {
    let h = read_header(&mut Cursor::new(bytes), Path::new(PATH)).unwrap();
    read_all_int(Cursor::new(bytes), &h.format, Path::new(PATH)).unwrap()
}

fn floats(bytes: &[u8]) -> Vec<f64> {
    let h = read_header(&mut Cursor::new(bytes), Path::new(PATH)).unwrap();
    read_all_float(Cursor::new(bytes), &h.format, Path::new(PATH)).unwrap()
}

fn wav(fmt_payload: &[u8], data: &[u8]) -> Vec<u8> {
    Form::riff()
        .chunk(b"fmt ", fmt_payload)
        .chunk(b"data", data)
        .build()
}

fn le(values: &[i32], width: usize) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| v.to_le_bytes().into_iter().take(width))
        .collect()
}

fn be(values: &[i32], width: usize) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| v.to_be_bytes().into_iter().skip(4 - width))
        .collect()
}

#[test]
fn wav_eight_bit_is_offset_binary() {
    let got = ints(&wav(&fmt_pcm(1, 8_000, 8), &[0, 128, 255, 1]));
    assert_eq!(got, [-128, 0, 127, -127]);
}

#[test]
fn wav_little_endian_integers_round_trip() {
    let v16 = [-32_768, 32_767, -1, 0, 1_234, -4_321];
    assert_eq!(ints(&wav(&fmt_pcm(2, 44_100, 16), &le(&v16, 2))), v16);
    let v24 = [-8_388_608, 8_388_607, -1, 0, 0x12_3456, -0x12_3456];
    assert_eq!(ints(&wav(&fmt_pcm(2, 44_100, 24), &le(&v24, 3))), v24);
    let v32 = [i32::MIN, i32::MAX, -1, 0];
    assert_eq!(ints(&wav(&fmt_pcm(1, 44_100, 32), &le(&v32, 4))), v32);
}

#[test]
fn valid_bits_below_the_container_are_shifted_down() {
    let v24 = [-8_388_608, 8_388_607, -1, 5];
    let stored: Vec<i32> = v24.iter().map(|v| v << 8).collect();
    let bytes = wav(&fmt_extensible(2, 48_000, 32, 24, 1), &le(&stored, 4));
    assert_eq!(ints(&bytes), v24);
    let v20 = [-524_288, 524_287, -1, 7];
    let stored: Vec<i32> = v20.iter().map(|v| v << 4).collect();
    assert_eq!(ints(&wav(&fmt(1, 2, 48_000, 6, 20), &le(&stored, 3))), v20);
}

fn aiff(channels: u16, bits: u16, frames: u32, audio: &[u8]) -> Vec<u8> {
    Form::aiff()
        .chunk(b"COMM", &comm(channels, frames, bits, 44_100, None))
        .chunk(b"SSND", &ssnd(4, audio))
        .build()
}

#[test]
fn aiff_big_endian_integers_round_trip() {
    let v8 = [-128, 127, -1, 0];
    assert_eq!(ints(&aiff(1, 8, 4, &be(&v8, 1))), v8);
    let v16 = [-32_768, 32_767, -1, 0];
    assert_eq!(ints(&aiff(2, 16, 2, &be(&v16, 2))), v16);
    let v24 = [-8_388_608, 8_388_607, -1, 0x12_3456];
    assert_eq!(ints(&aiff(2, 24, 2, &be(&v24, 3))), v24);
    let v32 = [i32::MIN, i32::MAX];
    assert_eq!(ints(&aiff(1, 32, 2, &be(&v32, 4))), v32);
    let v12 = [-2_048, 2_047, -1, 3];
    let stored: Vec<i32> = v12.iter().map(|v| v << 4).collect();
    assert_eq!(ints(&aiff(2, 12, 2, &be(&stored, 2))), v12);
}

#[test]
fn aifc_sowt_is_little_endian() {
    let v16 = [-32_768, 32_767, -2, 1];
    let bytes = Form::aifc()
        .chunk(b"COMM", &comm(2, 2, 16, 44_100, Some(b"sowt")))
        .chunk(b"SSND", &ssnd(0, &le(&v16, 2)))
        .build();
    assert_eq!(ints(&bytes), v16);
}

const FLOATS: [f64; 6] = [0.5, -1.0, 1.5, -0.0, 1e-30, -0.333_333_343_267_440_8];

/// The nearest `f32`: rounding is the point here.
#[allow(clippy::cast_possible_truncation)]
fn narrow(v: f64) -> f32 {
    v as f32
}

fn assert_bits(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g.to_bits(), w.to_bits(), "{g} != {w}");
    }
}

#[test]
fn float_wav_is_bit_exact() {
    let f32s: Vec<f32> = FLOATS.iter().map(|v| narrow(*v)).collect();
    let bytes: Vec<u8> = f32s.iter().flat_map(|v| v.to_le_bytes()).collect();
    let want: Vec<f64> = f32s.iter().map(|v| f64::from(*v)).collect();
    assert_bits(&floats(&wav(&fmt(3, 2, 44_100, 8, 32), &bytes)), &want);
    let bytes: Vec<u8> = FLOATS.iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_bits(&floats(&wav(&fmt(3, 2, 44_100, 16, 64), &bytes)), &FLOATS);
}

#[test]
fn float_aifc_is_bit_exact() {
    let aifc = |c: &[u8; 4], audio: &[u8]| {
        Form::aifc()
            .chunk(b"COMM", &comm(2, 3, 32, 44_100, Some(c)))
            .chunk(b"SSND", &ssnd(0, audio))
            .build()
    };
    let f32s: Vec<f32> = FLOATS.iter().map(|v| narrow(*v)).collect();
    let bytes: Vec<u8> = f32s.iter().flat_map(|v| v.to_be_bytes()).collect();
    let want: Vec<f64> = f32s.iter().map(|v| f64::from(*v)).collect();
    assert_bits(&floats(&aifc(b"fl32", &bytes)), &want);
    let bytes: Vec<u8> = FLOATS.iter().flat_map(|v| v.to_be_bytes()).collect();
    assert_bits(&floats(&aifc(b"fl64", &bytes)), &FLOATS);
}

#[test]
fn blocks_are_fixed_size_and_the_buffer_is_bounded() {
    let frames = BLOCK_FRAMES * 2 + 5;
    let values: Vec<i32> = (0..frames * 2)
        .map(|i| i32::try_from(i % 60_000).unwrap() - 30_000)
        .collect();
    let bytes = wav(&fmt_pcm(2, 44_100, 16), &le(&values, 2));
    let h = read_header(&mut Cursor::new(&bytes), Path::new(PATH)).unwrap();
    let mut pcm = PcmReader::new(Cursor::new(&bytes), &h.format, Path::new(PATH)).unwrap();
    assert_eq!(pcm.buffer_len_bytes(), BLOCK_FRAMES * 4);
    let mut out = Vec::new();
    let mut sizes = Vec::new();
    let mut all = Vec::new();
    loop {
        let n = pcm.next_block_int(&mut out).unwrap();
        assert_eq!(out.len(), n * 2);
        all.extend_from_slice(&out);
        sizes.push(n);
        if n == 0 {
            break;
        }
    }
    assert_eq!(sizes, [BLOCK_FRAMES, BLOCK_FRAMES, 5, 0]);
    assert_eq!(all, values);
    assert_eq!(pcm.frames_remaining(), 0);

    let tiny = wav(&fmt_pcm(2, 44_100, 16), &[0; 12]);
    let h = read_header(&mut Cursor::new(&tiny), Path::new(PATH)).unwrap();
    let pcm = PcmReader::new(Cursor::new(&tiny), &h.format, Path::new(PATH)).unwrap();
    assert_eq!(pcm.buffer_len_bytes(), 12);
}

#[test]
fn the_wrong_sample_kind_is_refused() {
    let bytes = wav(&fmt_pcm(1, 44_100, 16), &[0; 4]);
    let h = read_header(&mut Cursor::new(&bytes), Path::new(PATH)).unwrap();
    let mut pcm = PcmReader::new(Cursor::new(&bytes), &h.format, Path::new(PATH)).unwrap();
    assert!(!pcm.is_float());
    let r = pcm.next_block_float(&mut Vec::new());
    assert!(matches!(r, Err(Error::InvalidArgument(_))));
    let bytes = wav(&fmt(3, 1, 44_100, 4, 32), &[0; 4]);
    let r = read_all_int(
        Cursor::new(&bytes),
        &read_header(&mut Cursor::new(&bytes), Path::new(PATH))
            .unwrap()
            .format,
        Path::new(PATH),
    );
    assert!(matches!(r, Err(Error::InvalidArgument(_))));
}

#[test]
fn inconsistent_formats_and_short_files_are_errors() {
    let bytes = wav(&fmt_pcm(2, 44_100, 16), &[0; 8]);
    let good = read_header(&mut Cursor::new(&bytes), Path::new(PATH))
        .unwrap()
        .format;
    let bad = [
        AudioFormat {
            block_align: 3,
            ..good.clone()
        },
        AudioFormat {
            valid_bits: 17,
            ..good.clone()
        },
        AudioFormat {
            valid_bits: 0,
            ..good.clone()
        },
        AudioFormat {
            channels: 0,
            ..good.clone()
        },
        AudioFormat {
            encoding: SampleEncoding::FloatLe32,
            ..good.clone()
        },
        AudioFormat {
            encoding: SampleEncoding::UnsignedInt8,
            ..good.clone()
        },
    ];
    for f in bad {
        let r = PcmReader::new(Cursor::new(&bytes), &f, Path::new(PATH));
        assert!(matches!(r, Err(Error::InvalidArgument(_))), "{f:?}");
    }
    // The file shrank after the walk: the read fails cleanly.
    let cut = &bytes[..bytes.len() - 3];
    let r = read_all_int(Cursor::new(cut), &good, Path::new(PATH));
    assert!(matches!(r, Err(Error::Corrupt { .. })));
}
