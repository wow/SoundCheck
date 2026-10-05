//! Unit tests of `crates/sc-io/src/iff/write.rs`: the written headers read back through the
//! crate's own reader, and hand-checked bytes.
use std::io::Cursor;
use std::path::Path;

use super::*;
use crate::iff::{SampleEncoding, read_header};

#[test]
fn fmt_is_plain_pcm() {
    let p = wave_fmt_pcm(2, 44_100, 24);
    assert_eq!(
        p,
        [
            1, 0, 2, 0, 0x44, 0xAC, 0, 0, 0x98, 0x09, 0x04, 0, 6, 0, 24, 0
        ]
    );
    assert_eq!(
        wave_fmt_pcm(1, 48_000, 16)[8..14],
        [0x00, 0x77, 0x01, 0, 2, 0]
    );
}

#[test]
fn comm_has_the_exact_extended_rate() {
    let p = aiff_comm(2, 20_000, 16, 48_000);
    assert_eq!(p[..8], [0, 2, 0, 0, 0x4E, 0x20, 0, 16]);
    assert_eq!(p[8..], [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn samples_encode_in_both_byte_orders() {
    let mut out = Vec::new();
    encode_samples(&[-2, 0x12_3456], 24, false, &mut out);
    assert_eq!(out, [0xFE, 0xFF, 0xFF, 0x56, 0x34, 0x12]);
    encode_samples(&[-2, 0x12_3456], 24, true, &mut out);
    assert_eq!(out, [0xFF, 0xFF, 0xFE, 0x12, 0x34, 0x56]);
    encode_samples(&[-32_768, 0x0102], 16, false, &mut out);
    assert_eq!(out, [0x00, 0x80, 0x02, 0x01]);
    encode_samples(&[-32_768, 0x0102], 16, true, &mut out);
    assert_eq!(out, [0x80, 0x00, 0x01, 0x02]);
}

fn file(container: OutContainer, fmt: (&[u8; 4], &[u8]), audio: (&[u8; 4], &[u8])) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, payload) in [fmt, audio] {
        let len = u32::try_from(payload.len()).expect("small");
        body.extend_from_slice(&chunk_header(container, *id, len));
        body.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            body.push(0);
        }
    }
    let size = u32::try_from(4 + body.len()).expect("small");
    let mut out = container_header(container, size).to_vec();
    out.extend(body);
    out
}

#[test]
fn written_headers_read_back() {
    let mut samples = Vec::new();
    encode_samples(&[1, -1, 2, -2, 3], 24, false, &mut samples);
    let wav = file(
        OutContainer::RiffWave,
        (b"fmt ", &wave_fmt_pcm(1, 44_100, 24)),
        (b"data", &samples),
    );
    let h = read_header(&mut Cursor::new(&wav), Path::new("t.wav")).expect("valid WAV");
    let f = h.format;
    assert_eq!(
        (f.sample_rate, f.channels, f.bits_per_sample),
        (44_100, 1, 24)
    );
    assert_eq!(
        (f.frames, f.format_tag, f.encoding),
        (5, Some(1), SampleEncoding::IntLe)
    );
    assert_eq!(h.table.chunks[1].pad, Some(0));

    encode_samples(&[1, -1, 2, -2], 16, true, &mut samples);
    let mut ssnd = vec![0; 8];
    ssnd.extend_from_slice(&samples);
    let aiff = file(
        OutContainer::FormAiff,
        (b"COMM", &aiff_comm(2, 2, 16, 48_000)),
        (b"SSND", &ssnd),
    );
    let h = read_header(&mut Cursor::new(&aiff), Path::new("t.aiff")).expect("valid AIFF");
    let f = h.format;
    assert_eq!(
        (f.sample_rate, f.channels, f.bits_per_sample),
        (48_000, 2, 16)
    );
    assert_eq!((f.frames, f.encoding), (2, SampleEncoding::IntBe));
}
