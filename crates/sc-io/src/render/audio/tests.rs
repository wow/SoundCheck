//! Unit tests of `crates/sc-io/src/render/audio.rs`: write errors name the output, the cancel
//! flag, signed peaks, the dither seed, and the head cut through `apply_iff`: snapped back to
//! the quietest frame within 1 ms (never later), faded in over 2 ms, positions shifted by the
//! cut actually made, nothing faded without a cut.
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::Error;

use super::super::tests::{FailingWriter, Run, mono16, table};
use super::*;
use crate::iff::test_build::{Form, comm, fmt_pcm, read_ints, ssnd};
use sc_core::RenderRequest;

fn job<'a>(format: &'a AudioFormat, cancel: &'a AtomicBool) -> AudioJob<'a> {
    AudioJob {
        input: Path::new("in.wav"),
        output: Path::new("out.wav"),
        format,
        trim_frames: 0,
        frames_out: format.frames,
        bits: 16,
        big_endian: false,
        gain_db: -1.0,
        seed: 1,
        cancel,
    }
}

#[test]
fn a_failing_write_names_the_output() {
    let wav = mono16(44_100, &[1, 2, 3, 4]);
    let (_, format) = table(&wav);
    let cancel = AtomicBool::new(false);
    let err = write_audio(
        Cursor::new(&wav),
        &mut FailingWriter,
        &job(&format, &cancel),
    )
    .expect_err("the write fails");
    match err {
        Error::Io { path, .. } => assert_eq!(path, Path::new("out.wav")),
        other => panic!("{other}"),
    }
    let mut out = Vec::new();
    let done = write_audio(Cursor::new(&wav), &mut out, &job(&format, &cancel)).expect("written");
    assert_eq!(done.pcm_hash, *blake3::hash(&out).as_bytes());
}

#[test]
fn the_cancel_flag_stops_every_pass() {
    let wav = mono16(44_100, &[1, 2, 3, 4]);
    let (_, format) = table(&wav);
    let cancel = AtomicBool::new(true);
    let mut out = Vec::new();
    let err = write_audio(Cursor::new(&wav), &mut out, &job(&format, &cancel)).expect_err("set");
    assert!(matches!(err, Error::Cancelled));
    assert!(
        out.is_empty(),
        "{} bytes written after the cancel",
        out.len()
    );
    let p = Path::new("t");
    assert!(matches!(
        peaks_after_trim(Cursor::new(&wav), p, &format, 0, &cancel),
        Err(Error::Cancelled)
    ));
    let req = SeedRequest {
        gain_db: -1.0,
        trim_frames: 0,
        bits: 16,
    };
    assert!(matches!(
        dither_seed(Cursor::new(&wav), p, &format, &[], req, &cancel),
        Err(Error::Cancelled)
    ));
}

#[test]
fn peaks_are_signed_and_skip_the_trim() {
    let wav = mono16(44_100, &[32_767, -32_768, 16_384, -8_192]);
    let (_, format) = table(&wav);
    let off = AtomicBool::new(false);
    let p = Path::new("t");
    let all = peaks_after_trim(Cursor::new(&wav), p, &format, 0, &off).expect("read");
    assert_eq!((all.max, all.min), (32_767.0 / 32_768.0, -1.0));
    let after = peaks_after_trim(Cursor::new(&wav), p, &format, 2, &off).expect("read");
    assert_eq!((after.max, after.min), (0.5, -0.25));
}

#[test]
fn the_seed_depends_on_the_audio_and_the_request_only() {
    let samples: Vec<i16> = (0..70_000)
        .map(|i| i16::try_from(i % 3000).expect("small"))
        .collect();
    let wav = mono16(44_100, &samples);
    let (_, format) = table(&wav);
    let off = AtomicBool::new(false);
    let p = Path::new("t");
    let req = SeedRequest {
        gain_db: -3.2,
        trim_frames: 0,
        bits: 16,
    };
    let seed = |wav: &[u8], format: &AudioFormat, req: SeedRequest| {
        dither_seed(Cursor::new(wav), p, format, b"fmt", req, &off).expect("read")
    };
    let base = seed(&wav, &format, req);
    assert_eq!(base, seed(&wav, &format, req), "deterministic");
    assert_ne!(
        base,
        seed(
            &wav,
            &format,
            SeedRequest {
                gain_db: -3.1,
                ..req
            }
        )
    );
    assert_ne!(
        base,
        seed(
            &wav,
            &format,
            SeedRequest {
                trim_frames: 1,
                ..req
            }
        )
    );
    assert_ne!(base, seed(&wav, &format, SeedRequest { bits: 24, ..req }));
    // A change inside the first 65,536 frames changes the seed; one after them does not (but
    // the frame count is hashed, so a longer file does).
    let mut early = samples.clone();
    early[100] += 1;
    let mut late = samples.clone();
    late[66_000] += 1;
    assert_ne!(base, seed(&mono16(44_100, &early), &format, req));
    assert_eq!(base, seed(&mono16(44_100, &late), &format, req));
    let longer = mono16(44_100, &[samples.as_slice(), &[0]].concat());
    let (_, longer_format) = table(&longer);
    assert_ne!(base, seed(&longer, &longer_format, req));
}

/// `frames` frames of a 100 Hz sine at 20,000 (16-bit) that is exactly 0 at frame `zero`, on
/// `channels` channels (the second inverted), interleaved.
fn sine_zero_at(rate: u32, channels: usize, frames: usize, zero: usize) -> Vec<i16> {
    let mut v = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        #[allow(clippy::cast_precision_loss)]
        let t = (n as f64 - zero as f64) / f64::from(rate);
        // Inside +/-20,000: the cast is exact after rounding.
        #[allow(clippy::cast_possible_truncation)]
        let x = (20_000.0 * (std::f64::consts::TAU * 100.0 * t).sin()).round() as i16;
        for c in 0..channels {
            v.push(if c == 0 { x } else { -x });
        }
    }
    v
}

fn wav16(rate: u32, channels: u16, samples: &[i16], extra: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut form = Form::riff().chunk(b"fmt ", &fmt_pcm(channels, rate, 16));
    for (id, payload) in extra {
        form = form.chunk(id, payload);
    }
    form.chunk(b"data", &data).build()
}

fn cut(trim_frames: u64) -> RenderRequest {
    RenderRequest {
        trim_frames,
        ..RenderRequest::default()
    }
}

#[test]
fn trim_snaps_back_never_forward() {
    // A zero crossing 18 frames (0.41 ms at 44.1 kHz, 0.38 ms at 48 kHz) before the request,
    // and a silent frame 3 frames after it that must not be taken.
    for (rate, requested) in [(44_100_u32, 4410_usize), (48_000, 4800)] {
        let zero = requested - 18;
        let mut samples = sine_zero_at(rate, 2, 9000, zero);
        samples[2 * (requested + 3)..2 * (requested + 4)].fill(0);
        let run = Run::new(&wav16(rate, 2, &samples, &[]), "wav");
        let report = run.apply(&cut(requested as u64)).expect("render");
        assert_eq!(
            (report.trim_frames, report.trim_requested_frames),
            (zero as u64, requested as u64),
            "{rate} Hz"
        );
        assert_eq!(report.frames_out, 9000 - zero as u64);
        let (got, _) = read_ints(&run.out()).expect("readable");
        assert_eq!(got.len(), 2 * (9000 - zero));
        assert_eq!(got[..2], [0, 0], "the output starts on the zero crossing");
        let fade = crate::render::head_fade_frames(rate) * 2;
        let after: Vec<i32> = samples[2 * zero + fade..]
            .iter()
            .map(|x| i32::from(*x))
            .collect();
        assert_eq!(got[fade..], after[..], "unchanged after the fade");
    }
    // No quieter frame: a constant level keeps the requested cut (ties go to the latest).
    let run = Run::new(&wav16(44_100, 2, &[12_000; 2 * 3000], &[]), "wav");
    let report = run.apply(&cut(1000)).expect("render");
    assert_eq!((report.trim_frames, report.frames_out), (1000, 2000));
}

#[test]
fn fade_in_2ms_monotone() {
    for (rate, n, channels) in [(44_100_u32, 88_usize, 2_u16), (48_000, 96, 1)] {
        let ch = usize::from(channels);
        let run = Run::new(&wav16(rate, channels, &vec![10_000; ch * 4000], &[]), "wav");
        let report = run.apply(&cut(1000)).expect("render");
        assert_eq!(report.trim_frames, 1000);
        assert!(!report.exact && !report.dithered, "rounded, not dithered");
        let (got, _) = read_ints(&run.out()).expect("readable");
        let frames: Vec<i32> = got.chunks(ch).map(|f| f[0]).collect();
        for f in got.chunks(ch) {
            assert!(
                f.iter().all(|x| *x == f[0]),
                "the same gain on every channel"
            );
        }
        assert_eq!(frames[0], 0, "the first sample is silent");
        assert!(frames.windows(2).all(|w| w[1] >= w[0]), "monotone");
        for (i, y) in frames[..n].iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let w = 0.5 * (1.0 - (std::f64::consts::PI * i as f64 / n as f64).cos());
            assert!(
                (f64::from(*y) - 10_000.0 * w).abs() <= 0.5,
                "frame {i}: {y}"
            );
        }
        assert!(frames[n - 1] < 10_000, "the ramp is {n} frames long");
        assert!(
            frames[n..].iter().all(|y| *y == 10_000),
            "unity after {n} frames"
        );
    }
}

/// A `cue ` chunk with points at `positions` (`dwPosition` and `dwSampleOffset` both set).
fn cue(positions: &[u32]) -> Vec<u8> {
    let mut p = u32::try_from(positions.len())
        .expect("few")
        .to_le_bytes()
        .to_vec();
    for (i, pos) in positions.iter().enumerate() {
        p.extend_from_slice(&u32::try_from(i + 1).expect("few").to_le_bytes());
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

/// The payload of the first chunk `id` of `bytes`.
fn payload(bytes: &[u8], id: [u8; 4]) -> Vec<u8> {
    let (t, _) = table(bytes);
    let c = t.chunks.iter().find(|c| c.id == id).expect("chunk");
    let r = usize::try_from(c.payload.start).expect("small")
        ..usize::try_from(c.payload.end).expect("small");
    bytes[r].to_vec()
}

#[test]
fn positions_shift_by_actual_trim() {
    let (requested, zero) = (4410_u64, 4392_u32);
    let samples = sine_zero_at(44_100, 2, 12_000, zero as usize);
    let mut bext = vec![0_u8; 602];
    bext[338..346].copy_from_slice(&1000_u64.to_le_bytes());
    bext[346] = 1;
    let extra: [(&[u8], Vec<u8>); 2] = [(b"bext", bext), (b"cue ", cue(&[3000, 10_000]))];
    let run = Run::new(&wav16(44_100, 2, &samples, &extra), "wav");
    let report = run.apply(&cut(requested)).expect("render");
    assert_eq!(report.trim_frames, u64::from(zero));
    let out = run.out();
    let c = payload(&out, *b"cue ");
    assert_eq!((le32(&c, 8), le32(&c, 24)), (0, 0), "clamped at 0");
    assert_eq!((le32(&c, 32), le32(&c, 48)), (10_000 - zero, 10_000 - zero));
    let b = payload(&out, *b"bext");
    let time = u64::from_le_bytes(b[338..346].try_into().expect("8 bytes"));
    assert_eq!(
        time,
        1000 + u64::from(zero),
        "TimeReference plus the cut made"
    );
    // AIFF MARK: marker 1 at 8,000, name "A".
    let be: Vec<u8> = samples.iter().flat_map(|s| s.to_be_bytes()).collect();
    let mut mark = vec![0, 1, 0, 1];
    mark.extend_from_slice(&8000_u32.to_be_bytes());
    mark.extend_from_slice(&[1, b'A']);
    let aiff = Form::aiff()
        .chunk(b"COMM", &comm(2, 12_000, 16, 44_100, None))
        .chunk(b"MARK", &mark)
        .chunk(b"SSND", &ssnd(0, &be))
        .build();
    let run = Run::new(&aiff, "aiff");
    let report = run.apply(&cut(requested)).expect("render");
    assert_eq!(report.trim_frames, u64::from(zero));
    let m = payload(&run.out(), *b"MARK");
    let at = u32::from_be_bytes(m[4..8].try_into().expect("4 bytes"));
    assert_eq!(at, 8000 - zero);
}

#[test]
fn no_fade_without_trim() {
    let loud = vec![12_000_i16; 2 * 2000];
    let wav = wav16(44_100, 2, &loud, &[]);
    let run = Run::new(&wav, "wav");
    let report = run.apply(&cut(0)).expect("render");
    assert_eq!(run.out(), wav, "byte for byte, the first sample included");
    assert!(report.exact);
    assert_eq!((report.trim_frames, report.trim_requested_frames), (0, 0));
    // A cut whose quietest frame is frame 0 cuts nothing, so nothing is faded either.
    let mut head = loud.clone();
    head[..2].fill(0);
    let wav = wav16(44_100, 2, &head, &[]);
    let run = Run::new(&wav, "wav");
    let report = run.apply(&cut(20)).expect("render");
    assert_eq!((report.trim_frames, report.trim_requested_frames), (0, 20));
    assert_eq!((report.frames_out, report.exact), (2000, true));
    assert_eq!(run.out(), wav);
}
