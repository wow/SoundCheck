//! Unit tests of `crates/sc-io/src/render/audio.rs`: write errors name the output, the cancel
//! flag, signed peaks, and the dither seed.
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::Error;

use super::super::tests::{FailingWriter, mono16, table};
use super::*;

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
