//! Unit tests of `crates/sc-io/src/render.rs`: identity, report, container rules, and every
//! refusal leaving no output file, cancellation, and which path an I/O error names. Planning
//! is tested in `layout/tests.rs`, the audio stage in `audio/tests.rs`; the fixture matrix
//! (`tests/matrix.rs`) covers the full chunk-by-chunk contract.
#![allow(clippy::float_cmp)] // exact values are intended in these tests

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, TagEdit};

use super::*;
use crate::iff::test_build::{Form, comm, fmt_extensible, fmt_pcm, read_ints, ssnd};
use crate::iff::{ChunkTable, read_header};

/// 16-bit little-endian samples: a ramp over both channels.
fn pcm16(frames: usize, channels: usize) -> (Vec<i32>, Vec<u8>) {
    let samples: Vec<i32> = (0..frames * channels)
        .map(|i| i32::try_from(i % 2000).expect("small") * 7 - 7000)
        .collect();
    let bytes = samples
        .iter()
        .flat_map(|s| i16::try_from(*s).expect("16-bit").to_le_bytes())
        .collect();
    (samples, bytes)
}

struct Run {
    _dir: tempfile::TempDir,
    input: PathBuf,
    output: PathBuf,
}

impl Run {
    fn new(bytes: &[u8], ext: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join(format!("in.{ext}"));
        let output = dir.path().join(format!("out.{ext}"));
        std::fs::write(&input, bytes).expect("write input");
        Self {
            _dir: dir,
            input,
            output,
        }
    }

    fn apply(&self, req: &RenderRequest) -> Result<RenderReport> {
        apply_iff(&self.input, &self.output, req, &AtomicBool::new(false))
    }

    fn out(&self) -> Vec<u8> {
        std::fs::read(&self.output).expect("output written")
    }

    /// Asserts `req` is refused with an error `is` accepts and leaves no output.
    fn refused(&self, req: &RenderRequest, is: impl Fn(&Error) -> bool) {
        let _ = std::fs::remove_file(&self.output);
        match self.apply(req) {
            Ok(_) => panic!("{req:?} was not refused"),
            Err(e) => assert!(is(&e), "unexpected error {e}"),
        }
        assert!(!self.output.exists(), "a refusal left an output file");
    }
}

fn gain(gain_db: f64) -> RenderRequest {
    RenderRequest {
        gain_db,
        ..RenderRequest::default()
    }
}

pub(super) fn table(bytes: &[u8]) -> (ChunkTable, crate::iff::AudioFormat) {
    let h = read_header(&mut Cursor::new(bytes), Path::new("t")).expect("valid file");
    (h.table, h.format)
}

#[test]
fn a_canonical_wav_at_0_db_is_rewritten_byte_for_byte() {
    let (_, data) = pcm16(1000, 2);
    let wav = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"LIST", b"INFOINAM\x05\x00\x00\x00Tone\x00\x00")
        .chunk(b"data", &data)
        .build();
    let run = Run::new(&wav, "wav");
    let report = run.apply(&RenderRequest::default()).expect("render");
    assert_eq!(run.out(), wav);
    assert!(report.exact && !report.dithered);
    assert_eq!(
        (report.frames_in, report.frames_out, report.bits_out),
        (1000, 1000, 16)
    );
    assert_eq!(report.pcm_hash, *blake3::hash(&data).as_bytes());
    assert_eq!(report.output_bytes, wav.len() as u64);
    assert_eq!(
        (
            report.count(BlockFate::Replaced),
            report.count(BlockFate::Carried)
        ),
        (2, 1)
    );
}

#[test]
fn extensible_source_gets_a_plain_header_and_its_chunks_keep_their_pads() {
    // 24-bit extensible, a JUNK chunk, a PCM fact, an odd chunk padded with a space, and an
    // odd chunk at the very end without its pad byte.
    let samples: Vec<i32> = (0..2000).map(|i| (i - 1000) * 4099).collect();
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| s.to_le_bytes()[..3].to_vec())
        .collect();
    let wav = Form::riff()
        .chunk(b"JUNK", &[0; 28])
        .chunk(b"fmt ", &fmt_extensible(2, 48_000, 24, 24, 1))
        .chunk(b"fact", &1000_u32.to_le_bytes())
        .chunk_pad(b"xodd", b"odd", Some(0x20))
        .chunk(b"data", &data)
        .chunk_pad(b"last", b"end", None)
        .build();
    let run = Run::new(&wav, "wav");
    let req = RenderRequest {
        gain_db: -3.2,
        trim_frames: 10,
        ..RenderRequest::default()
    };
    let report = run.apply(&req).expect("render");
    let out = run.out();
    let (t, f) = table(&out);
    let ids: Vec<_> = t.chunks.iter().map(|c| c.id).collect();
    assert_eq!(
        ids,
        [*b"JUNK", *b"fmt ", *b"fact", *b"xodd", *b"data", *b"last"]
    );
    assert_eq!(
        (f.format_tag, f.bits_per_sample, f.frames),
        (Some(1), 24, 990)
    );
    assert_eq!(t.chunks[1].payload_len(), 16);
    let fact = &t.chunks[2].payload;
    assert_eq!(
        out[usize::try_from(fact.start).expect("small")..usize::try_from(fact.end).expect("small")],
        990_u32.to_le_bytes()
    );
    assert_eq!(t.chunks[3].pad, Some(0x20));
    assert_eq!(t.chunks[5].pad, Some(0), "a missing pad is written as 0");
    assert!(t.trailing.is_none() && !t.chunks.iter().any(|c| c.beyond_container));
    assert_eq!(report.count(BlockFate::Patched), 1);
    assert!(!report.exact && !report.dithered);
    // The first output sample is input sample 10 (frame 5... of 2 channels: index 20) after gain.
    let (got, _) = read_ints(&out).expect("readable");
    let g = 10_f64.powf(-3.2 / 20.0);
    let want = (f64::from(samples[20]) * g).round_ties_even();
    assert_eq!(f64::from(got[0]), want);
}

#[test]
fn aifc_sowt_becomes_big_endian_aiff_with_identical_samples() {
    let (samples, le) = pcm16(500, 2);
    let aifc = Form::aifc()
        .chunk(b"FVER", &0xA280_5140_u32.to_be_bytes())
        .chunk(b"COMM", &comm(2, 500, 16, 44_100, Some(b"sowt")))
        .chunk(b"SSND", &ssnd(4, &le))
        .build();
    let run = Run::new(&aifc, "aifc");
    let report = run.apply(&RenderRequest::default()).expect("render");
    let out = run.out();
    assert_eq!(&out[8..12], b"AIFF");
    let (t, f) = table(&out);
    assert_eq!(
        t.chunks.iter().map(|c| c.id).collect::<Vec<_>>(),
        [*b"COMM", *b"SSND"]
    );
    assert_eq!(t.chunks[0].payload_len(), 18);
    assert_eq!(f.data.start, t.chunks[1].payload.start + 8, "SSND offset 0");
    assert_eq!(read_ints(&out).expect("readable").0, samples);
    assert_eq!(report.count(BlockFate::Dropped), 1);
    assert!(report.exact);
}

#[test]
fn chunks_past_a_stale_container_size_move_inside_and_trailing_bytes_stay_last() {
    let (_, data) = pcm16(100, 1);
    let form = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk(b"data", &data);
    let stale = u32::try_from(4 + form.body.len()).expect("small");
    let mut wav = form.chunk(b"id3 ", b"ID3\x04tag").build_with_size(stale);
    wav.extend_from_slice(b"\xFFstray");
    let (t, _) = table(&wav);
    assert!(t.chunks[2].beyond_container);
    let run = Run::new(&wav, "wav");
    let report = run.apply(&gain(-1.0)).expect("render");
    let out = run.out();
    let (t, _) = table(&out);
    assert!(!t.chunks.iter().any(|c| c.beyond_container));
    assert_eq!(t.chunks[2].id, *b"id3 ");
    assert_eq!(&out[out.len() - 6..], b"\xFFstray");
    assert_eq!(report.trailing_bytes, 6);
}

#[test]
fn renders_are_deterministic_and_dither_only_at_16_bit() {
    let (_, data) = pcm16(3000, 2);
    let wav = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &data)
        .build();
    let run = Run::new(&wav, "wav");
    let a = run.apply(&gain(-3.2)).expect("render");
    let first = run.out();
    std::fs::remove_file(&run.output).expect("remove");
    let b = run.apply(&gain(-3.2)).expect("render");
    assert_eq!(run.out(), first);
    assert_eq!(a, b);
    assert!(a.dithered);
    std::fs::remove_file(&run.output).expect("remove");
    let up = RenderRequest {
        bits: Some(24),
        ..gain(-3.2)
    };
    assert!(!run.apply(&up).expect("render").dithered);
}

#[test]
fn an_existing_output_is_never_overwritten() {
    let (_, data) = pcm16(10, 1);
    let wav = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk(b"data", &data)
        .build();
    let run = Run::new(&wav, "wav");
    let err = apply_iff(&run.input, &run.input, &gain(-1.0), &AtomicBool::new(false))
        .expect_err("the input exists");
    assert!(matches!(err, Error::Io { .. }));
    assert_eq!(std::fs::read(&run.input).expect("input"), wav);
}

pub(super) fn mono16(rate: u32, samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, rate, 16))
        .chunk(b"data", &data)
        .build()
}

#[test]
fn refusals_name_their_reason_and_leave_no_output() {
    let run = Run::new(&mono16(96_000, &[0; 8]), "wav");
    run.refused(
        &gain(0.0),
        |e| matches!(e, Error::NotDjSafe { reason, .. } if reason.contains("96000")),
    );

    let three = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(3, 44_100, 16))
        .chunk(b"data", &[0; 12])
        .build();
    Run::new(&three, "wav").refused(&gain(0.0), |e| {
        matches!(e, Error::UnsupportedChannels { channels: 3, .. })
    });

    // Full-scale negative sample: any boost would clip; 0 dB and cuts are fine.
    let run = Run::new(&mono16(44_100, &[0, -32_768, 100]), "wav");
    run.refused(&gain(0.1), |e| matches!(e, Error::WouldClip { .. }));
    run.apply(&gain(-0.1)).expect("a cut never clips");

    let run = Run::new(&mono16(44_100, &[0; 8]), "wav");
    run.refused(
        &RenderRequest {
            trim_frames: 8,
            ..gain(0.0)
        },
        |e| matches!(e, Error::InvalidArgument(m) if m.contains("leaves no audio")),
    );
    run.refused(
        &RenderRequest {
            bits: Some(20),
            ..gain(0.0)
        },
        |e| matches!(e, Error::InvalidArgument(_)),
    );
    run.refused(&gain(f64::NAN), |e| matches!(e, Error::InvalidArgument(_)));
    let edits = vec![TagEdit {
        label: "APIC".into(),
        value: "128".into(),
    }];
    run.refused(
        &RenderRequest {
            tag_edits: edits,
            ..gain(0.0)
        },
        |e| matches!(e, Error::InvalidArgument(m) if m.contains("APIC")),
    );

    let two_data = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk(b"data", &[0; 4])
        .chunk(b"data", &[0; 4])
        .build();
    Run::new(&two_data, "wav").refused(
        &gain(0.0),
        |e| matches!(e, Error::UnsupportedFormat { detail, .. } if detail.contains("second")),
    );

    let mut cut = mono16(44_100, &[1; 100]);
    cut.truncate(cut.len() - 10);
    Run::new(&cut, "wav").refused(&gain(0.0), |e| matches!(e, Error::Corrupt { .. }));
}

#[test]
fn float_peaks_at_full_scale_and_data_in_padding_bits_are_refused() {
    let float = |peak: f32| {
        let data: Vec<u8> = [0.25_f32, peak, 0.5]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        Form::riff()
            .chunk(b"fmt ", &crate::iff::test_build::fmt(3, 1, 44_100, 4, 32))
            .chunk(b"fact", &3_u32.to_le_bytes())
            .chunk(b"data", &data)
            .build()
    };
    // +1.0 is past the top code; -1.0 is the bottom code itself and stays.
    let run = Run::new(&float(1.0), "wav");
    run.refused(
        &gain(0.0),
        |e| matches!(e, Error::WouldClip { over_db, .. } if *over_db == 0.0),
    );
    let report = run
        .apply(&gain(-0.5))
        .expect("below full scale after the cut");
    assert_eq!((report.bits_out, report.count(BlockFate::Dropped)), (24, 1));
    let run = Run::new(&float(-1.0), "wav");
    let report = run.apply(&gain(0.0)).expect("-1.0 is a valid code");
    assert_eq!(report.samples_saturated, 0);
    assert_eq!(read_ints(&run.out()).expect("readable").0[1], -8_388_608);
    std::fs::remove_file(&run.output).expect("remove");
    run.refused(&gain(0.01), |e| matches!(e, Error::WouldClip { .. }));
    let nan = Run::new(&float(f32::NAN), "wav");
    nan.refused(&gain(-6.0), |e| matches!(e, Error::Corrupt { .. }));
    // The largest float below full scale, 1 - 2^-24, is 8388607.5 at 24 bits: the tie rounds
    // to even, one step past the top code, so it is saturated and counted; the negative one
    // lands on the bottom code.
    let top = 1.0 - f32::EPSILON / 2.0;
    let report = Run::new(&float(top), "wav")
        .apply(&RenderRequest::default())
        .expect("below full scale");
    assert_eq!(report.samples_saturated, 1);
    let report = Run::new(&float(-top), "wav")
        .apply(&RenderRequest::default())
        .expect("below full scale");
    assert_eq!(report.samples_saturated, 0);

    // 24 valid bits in a 32-bit container whose low byte holds data.
    let data: Vec<u8> = [0x0100_0001_i32, 0x0200_0000]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let padded = Form::riff()
        .chunk(b"fmt ", &fmt_extensible(1, 44_100, 32, 24, 1))
        .chunk(b"data", &data)
        .build();
    Run::new(&padded, "wav").refused(
        &gain(-1.0),
        |e| matches!(e, Error::UnsupportedFormat { detail, .. } if detail.contains("padding bits")),
    );
}

#[test]
fn a_set_cancel_flag_stops_the_render_and_leaves_nothing() {
    let (_, data) = pcm16(10_000, 2);
    let wav = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, 44_100, 16))
        .chunk(b"data", &data)
        .build();
    let run = Run::new(&wav, "wav");
    for req in [gain(-3.2), gain(1.0)] {
        let err = apply_iff(&run.input, &run.output, &req, &AtomicBool::new(true))
            .expect_err("cancelled");
        assert!(matches!(err, Error::Cancelled), "{err}");
        assert!(!run.output.exists());
    }
}

/// A writer whose every write fails.
pub(super) struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("disk full"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A source whose reads fail after the seek.
struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("bad sector"))
    }
}

impl Seek for FailingReader {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Ok(0)
    }
}

#[test]
fn copy_errors_name_the_side_that_failed() {
    let (input, output) = (Path::new("in.wav"), Path::new("out.wav"));
    let named = |e: &Error| match e {
        Error::Io { path, .. } | Error::Corrupt { path, .. } => path.clone(),
        other => panic!("unexpected {other}"),
    };
    let mut src = Cursor::new(vec![7_u8; 100]);
    let err = copy_range(&mut src, input, output, &(10..90), &mut FailingWriter)
        .expect_err("the write fails");
    assert!(matches!(err, Error::Io { .. }));
    assert_eq!(named(&err), output);
    let err = copy_range(&mut FailingReader, input, output, &(0..10), &mut Vec::new())
        .expect_err("the read fails");
    assert!(matches!(err, Error::Io { .. }));
    assert_eq!(named(&err), input);
    let err = copy_range(&mut src, input, output, &(90..120), &mut Vec::new())
        .expect_err("the source is short");
    assert!(matches!(err, Error::Corrupt { .. }));
    assert_eq!(named(&err), input);
    let mut out = Vec::new();
    copy_range(&mut src, input, output, &(10..90), &mut out).expect("copied");
    assert_eq!(out, vec![7; 80]);
}

#[test]
fn the_output_guard_removes_the_file_on_error_and_panic_unless_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("out.wav");
    std::fs::write(&path, b"partial").expect("write");
    drop(OutputGuard::new(&path));
    assert!(!path.exists(), "dropped without keep: removed");
    std::fs::write(&path, b"complete").expect("write");
    OutputGuard::new(&path).keep();
    assert!(path.exists(), "kept");
    let panicked = std::panic::catch_unwind(|| {
        let _guard = OutputGuard::new(&path);
        panic!("a bug while writing");
    });
    assert!(panicked.is_err());
    assert!(!path.exists(), "a panic removes the partial output");
}
