use std::io::Cursor;
use std::path::Path;

use sc_core::Error;

use super::super::test_build::{Form, comm, fmt, fmt_extensible, fmt_pcm, ssnd};
use super::super::{IffHeader, read_header};
use super::*;

fn header(bytes: &[u8]) -> sc_core::Result<IffHeader> {
    read_header(&mut Cursor::new(bytes), Path::new("t.wav"))
}

fn format_of(bytes: &[u8]) -> AudioFormat {
    header(bytes).unwrap().format
}

fn wav(fmt_payload: &[u8], data: &[u8]) -> Vec<u8> {
    Form::riff()
        .chunk(b"fmt ", fmt_payload)
        .chunk(b"data", data)
        .build()
}

fn is_corrupt(r: &sc_core::Result<IffHeader>) -> bool {
    matches!(r, Err(Error::Corrupt { .. }))
}

fn is_unsupported(r: &sc_core::Result<IffHeader>) -> bool {
    matches!(r, Err(Error::UnsupportedFormat { .. }))
}

#[test]
fn plain_pcm_wav() {
    let f = format_of(&wav(&fmt_pcm(2, 44_100, 16), &[0; 40]));
    assert_eq!(f.sample_rate, 44_100);
    assert_eq!((f.channels, f.bits_per_sample, f.valid_bits), (2, 16, 16));
    assert_eq!(
        (f.encoding, f.block_align, f.frames),
        (SampleEncoding::IntLe, 4, 10)
    );
    assert_eq!(f.data, 44..84);
    assert_eq!((f.format_tag, f.channel_mask), (Some(1), None));
    assert!(!f.frames_mismatch);
}

#[test]
fn eight_bit_wav_is_unsigned_and_a_partial_frame_is_excluded() {
    let f = format_of(&wav(&fmt_pcm(2, 22_050, 8), &[128; 7]));
    assert_eq!(f.encoding, SampleEncoding::UnsignedInt8);
    assert_eq!((f.frames, f.data.end - f.data.start), (3, 6));
}

#[test]
fn twenty_bit_plain_pcm_sits_in_its_block_align_container() {
    let f = format_of(&wav(&fmt(1, 2, 48_000, 6, 20), &[0; 12]));
    assert_eq!((f.bits_per_sample, f.valid_bits, f.frames), (24, 20, 2));
    let f = format_of(&wav(&fmt(1, 1, 48_000, 4, 24), &[0; 12]));
    assert_eq!((f.bits_per_sample, f.valid_bits, f.frames), (32, 24, 3));
}

#[test]
fn extensible_pcm_with_valid_bits_and_channel_mask() {
    let f = format_of(&wav(&fmt_extensible(2, 96_000, 32, 24, 1), &[0; 16]));
    assert_eq!((f.bits_per_sample, f.valid_bits), (32, 24));
    assert_eq!(
        (f.encoding, f.block_align, f.frames),
        (SampleEncoding::IntLe, 8, 2)
    );
    assert_eq!((f.format_tag, f.channel_mask), (Some(0xFFFE), Some(3)));
    // Valid bits 0 means "the container width".
    let f = format_of(&wav(&fmt_extensible(2, 48_000, 24, 0, 1), &[0; 12]));
    assert_eq!(f.valid_bits, 24);
}

#[test]
fn float_wav_plain_and_extensible() {
    let f = format_of(&wav(&fmt(3, 2, 44_100, 8, 32), &[0; 16]));
    assert_eq!((f.encoding, f.frames), (SampleEncoding::FloatLe32, 2));
    let f = format_of(&wav(&fmt(3, 1, 44_100, 8, 64), &[0; 16]));
    assert_eq!(
        (f.encoding, f.valid_bits, f.frames),
        (SampleEncoding::FloatLe64, 64, 2)
    );
    let f = format_of(&wav(&fmt_extensible(2, 44_100, 32, 32, 3), &[0; 16]));
    assert_eq!(f.encoding, SampleEncoding::FloatLe32);
}

#[test]
fn other_wav_encodings_are_unsupported() {
    // IMA ADPCM, MPEG Layer 3, A-law.
    for tag in [0x0011_u16, 0x0055, 0x0006] {
        assert!(is_unsupported(&header(&wav(
            &fmt(tag, 2, 44_100, 4, 16),
            &[0; 8]
        ))));
    }
    let mut ambisonic = fmt_extensible(2, 48_000, 16, 16, 1);
    ambisonic[36] = 0x01; // a GUID outside the KSDATAFORMAT family
    assert!(is_unsupported(&header(&wav(&ambisonic, &[0; 8]))));
    assert!(is_unsupported(&header(&wav(
        &fmt_extensible(2, 48_000, 16, 16, 2),
        &[0; 8]
    ))));
    assert!(is_unsupported(&header(&wav(
        &fmt(3, 2, 48_000, 4, 16),
        &[0; 8]
    ))));
    assert!(is_unsupported(&header(&wav(
        &fmt(1, 1, 48_000, 5, 40),
        &[0; 10]
    ))));
}

#[test]
fn impossible_wav_headers_are_corrupt() {
    assert!(is_corrupt(&header(&wav(&fmt_pcm(0, 44_100, 16), &[0; 8]))));
    assert!(is_corrupt(&header(&wav(&fmt_pcm(2, 0, 16), &[0; 8]))));
    assert!(is_corrupt(&header(&wav(
        &fmt_pcm(2, 10_000_000, 16),
        &[0; 8]
    ))));
    assert!(is_corrupt(&header(&wav(
        &fmt(1, 2, 44_100, 3, 16),
        &[0; 8]
    ))));
    assert!(is_corrupt(&header(&wav(
        &fmt(1, 2, 44_100, 12, 16),
        &[0; 8]
    ))));
    assert!(is_corrupt(&header(&wav(
        &fmt_extensible(2, 48_000, 16, 20, 1),
        &[0; 8]
    ))));
    assert!(is_corrupt(&header(&wav(
        &fmt_pcm(2, 44_100, 16)[..14],
        &[0; 8]
    ))));
    assert!(is_corrupt(&header(&wav(
        &fmt_extensible(2, 48_000, 16, 16, 1)[..30],
        &[0; 8]
    ))));
    let no_data = Form::riff().chunk(b"fmt ", &fmt_pcm(2, 44_100, 16)).build();
    assert!(is_corrupt(&header(&no_data)));
    let no_fmt = Form::riff().chunk(b"data", &[0; 4]).build();
    assert!(is_corrupt(&header(&no_fmt)));
}

fn aiff(comm_payload: &[u8], ssnd_payload: &[u8]) -> Vec<u8> {
    Form::aiff()
        .chunk(b"COMM", comm_payload)
        .chunk(b"SSND", ssnd_payload)
        .build()
}

#[test]
fn aiff_with_an_ssnd_offset() {
    let bytes = aiff(&comm(1, 3, 16, 44_100, None), &ssnd(12, &[0; 6]));
    let f = format_of(&bytes);
    assert_eq!(
        (f.sample_rate, f.channels, f.encoding),
        (44_100, 1, SampleEncoding::IntBe)
    );
    assert_eq!(
        (f.frames, f.frames_declared, f.frames_mismatch),
        (3, Some(3), false)
    );
    // FORM header 12, COMM 8 + 18, SSND header 8, fields 8, offset 12.
    let start = 12 + 26 + 8 + 8 + 12;
    assert_eq!(f.data, start..start + 6);
    assert_eq!(f.aifc_compression, None);
}

#[test]
fn aiff_frame_count_disagreement_takes_the_smaller() {
    let f = format_of(&aiff(&comm(2, 10, 24, 48_000, None), &ssnd(0, &[0; 24])));
    assert_eq!(
        (f.frames, f.frames_declared, f.frames_mismatch),
        (4, Some(10), true)
    );
    let f = format_of(&aiff(&comm(2, 2, 24, 48_000, None), &ssnd(0, &[0; 24])));
    assert_eq!((f.frames, f.frames_mismatch), (2, true));
    assert_eq!(f.data.end - f.data.start, 12);
}

#[test]
fn aiff_odd_sample_sizes_are_left_justified() {
    let f = format_of(&aiff(&comm(2, 1, 12, 8_000, None), &ssnd(0, &[0; 4])));
    assert_eq!(
        (f.bits_per_sample, f.valid_bits, f.block_align),
        (16, 12, 4)
    );
}

#[test]
fn aifc_compression_types() {
    let aifc = |c: &[u8; 4], bits: u16| {
        let bytes = Form::aifc()
            .chunk(b"COMM", &comm(2, 1, bits, 44_100, Some(c)))
            .chunk(b"SSND", &ssnd(0, &[0; 16]))
            .build();
        header(&bytes)
    };
    let enc = |c: &[u8; 4], bits| aifc(c, bits).unwrap().format.encoding;
    assert_eq!(enc(b"NONE", 16), SampleEncoding::IntBe);
    assert_eq!(enc(b"twos", 16), SampleEncoding::IntBe);
    assert_eq!(enc(b"sowt", 16), SampleEncoding::IntLe);
    assert_eq!(enc(b"fl32", 32), SampleEncoding::FloatBe32);
    assert_eq!(enc(b"FL32", 32), SampleEncoding::FloatBe32);
    assert_eq!(enc(b"fl64", 64), SampleEncoding::FloatBe64);
    for refused in [b"able", b"ima4", b"ulaw", b"alaw", b"MAC3"] {
        assert!(is_unsupported(&aifc(refused, 16)), "{refused:?}");
    }
}

#[test]
fn a_refused_codec_is_named_in_the_error() {
    let mut c = comm(2, 1, 16, 44_100, Some(b"able"));
    c.truncate(22);
    c.extend_from_slice(b"\x07Ableton");
    let bytes = Form::aifc()
        .chunk(b"COMM", &c)
        .chunk(b"SSND", &ssnd(0, &[0; 4]))
        .build();
    let Err(Error::UnsupportedFormat { detail, .. }) = header(&bytes) else {
        panic!("able must be refused");
    };
    assert_eq!(detail, "AIFF-C compression 'able' (Ableton)");
}

#[test]
fn impossible_aiff_headers() {
    let mut fractional = comm(2, 1, 16, 44_100, None);
    fractional[8..18]
        .copy_from_slice(&[0x40, 0x0D, 0xAD, 0xDD, 0x17, 0x45, 0xD1, 0x74, 0x5D, 0x17]);
    assert!(is_unsupported(&header(&aiff(
        &fractional,
        &ssnd(0, &[0; 4])
    ))));
    assert!(is_unsupported(&header(&aiff(
        &comm(2, 1, 0, 44_100, None),
        &ssnd(0, &[0; 4])
    ))));
    assert!(is_unsupported(&header(&aiff(
        &comm(2, 1, 40, 44_100, None),
        &ssnd(0, &[0; 4])
    ))));
    assert!(is_corrupt(&header(&aiff(
        &comm(0, 1, 16, 44_100, None),
        &ssnd(0, &[0; 4])
    ))));
    assert!(is_corrupt(&header(&aiff(
        &comm(2, 1, 16, 44_100, None)[..17],
        &ssnd(0, &[0; 4])
    ))));
    assert!(is_corrupt(&header(&aiff(
        &comm(2, 1, 16, 500, None),
        &ssnd(0, &[0; 4])
    ))));
    assert!(is_corrupt(&header(&aiff(
        &comm(2, 1, 16, 44_100, None),
        &ssnd(9, &[0; 4])[..12]
    ))));
    assert!(is_corrupt(&header(&aiff(
        &comm(2, 1, 16, 44_100, None),
        &[0; 6]
    ))));
    let no_ssnd = Form::aiff()
        .chunk(b"COMM", &comm(2, 5, 16, 44_100, None))
        .build();
    assert!(is_corrupt(&header(&no_ssnd)));
    let no_comm = Form::aiff().chunk(b"SSND", &ssnd(0, &[0; 4])).build();
    assert!(is_corrupt(&header(&no_comm)));
    let aifc_short = Form::aifc()
        .chunk(b"COMM", &comm(2, 1, 16, 44_100, None))
        .chunk(b"SSND", &ssnd(0, &[0; 4]))
        .build();
    assert!(is_corrupt(&header(&aifc_short)));
}

#[test]
fn aiff_without_frames_needs_no_ssnd() {
    let bytes = Form::aiff()
        .chunk(b"COMM", &comm(2, 0, 16, 44_100, None))
        .build();
    let f = format_of(&bytes);
    // FORM header 12 + COMM header 8 + 18 bytes: an empty range at the end of COMM.
    assert_eq!((f.frames, f.data), (0, 38..38));
    assert_eq!((f.format_chunk, f.audio_chunk), (0, None));
}

#[test]
fn the_chunks_used_are_recorded_by_index() {
    let bytes = Form::riff()
        .chunk(b"JUNK", &[0; 4])
        .chunk(b"data", &[0; 8])
        .chunk(b"LIST", b"INFO")
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .build();
    let f = format_of(&bytes);
    assert_eq!((f.format_chunk, f.audio_chunk), (3, Some(1)));
    let bytes = aiff(&comm(1, 3, 16, 44_100, None), &ssnd(0, &[0; 6]));
    let f = format_of(&bytes);
    assert_eq!((f.format_chunk, f.audio_chunk), (0, Some(1)));
}
