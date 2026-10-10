//! Unit tests of `crates/sc-io/src/render/snap.rs`: the snap a caller gets is the render's, a
//! render of the snapped cut flagged as such is byte for byte the render of the original
//! request on every container, the snap is not idempotent (so the flag matters), and a flagged
//! cut outside the window is refused.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, TagEdit};

use super::*;
use crate::flac::test_build::{encode, file, vorbis};
use crate::iff::test_build::{Form, comm, fmt_pcm, ssnd};
use crate::render::{RenderReport, apply_flac, apply_iff};

const RATE: u32 = 44_100;
const FRAMES: usize = 20_000;
/// The requested cut; the window is 44 frames at 44.1 kHz.
const T: u64 = 5_000;

/// Stereo samples whose level rises with the frame index (left `i * step`, right `-i * step /
/// 2`), so the quietest frame of any window is its first: each snap moves the cut a full 1 ms
/// back.
fn rising(step: i32) -> Vec<i32> {
    (0..FRAMES)
        .flat_map(|i| {
            let x = i32::try_from(i).expect("small") * step;
            [x, -x / 2]
        })
        .collect()
}

fn le_bytes(samples: &[i32], bytes: usize) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| s.to_le_bytes()[..bytes].to_vec())
        .collect()
}

fn be_bytes(samples: &[i32], bytes: usize) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| s.to_be_bytes()[4 - bytes..].to_vec())
        .collect()
}

fn wav(bits: u16) -> Vec<u8> {
    let step = if bits == 16 { 1 } else { 256 };
    let data = le_bytes(&rising(step), usize::from(bits / 8));
    Form::riff()
        .chunk(b"fmt ", &fmt_pcm(2, RATE, bits))
        .chunk(b"data", &data)
        .build()
}

fn aiff() -> Vec<u8> {
    let frames = u32::try_from(FRAMES).expect("small");
    Form::aiff()
        .chunk(b"COMM", &comm(2, frames, 24, RATE, None))
        .chunk(b"SSND", &ssnd(0, &be_bytes(&rising(256), 3)))
        .build()
}

fn flac(bits: u8) -> Vec<u8> {
    let step = if bits == 16 { 1 } else { 256 };
    let (frames, info) = encode(&rising(step), RATE, 2, bits);
    file(
        &[],
        &info,
        &[(4, vorbis("ref", &["TITLE=t"]))],
        &frames,
        &[],
    )
}

/// A source file in a temporary folder.
struct Source {
    dir: tempfile::TempDir,
    path: PathBuf,
}

impl Source {
    fn new(bytes: &[u8], ext: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(format!("in.{ext}"));
        std::fs::write(&path, bytes).expect("write");
        Self { dir, path }
    }

    /// Renders `req` into a new file named `name`; its report and bytes.
    fn render(&self, req: &RenderRequest, name: &str) -> Result<(RenderReport, Vec<u8>)> {
        let out = self.dir.path().join(name);
        let cancel = AtomicBool::new(false);
        let flac = self.path.extension().is_some_and(|e| e == "flac");
        let report = if flac {
            apply_flac(&self.path, &out, req, &cancel)?
        } else {
            apply_iff(&self.path, &out, req, &cancel)?
        };
        let bytes = std::fs::read(&out).expect("output written");
        Ok((report, bytes))
    }
}

/// A gain, a cut and a tag edit, at 16 bits (dithered from a 24-bit source).
fn request(trim_frames: u64, trim_snapped_from: Option<u64>, label: &str) -> RenderRequest {
    RenderRequest {
        gain_db: -3.0,
        trim_frames,
        trim_snapped_from,
        bits: Some(16),
        loudness: None,
        tag_edits: vec![TagEdit {
            label: label.to_owned(),
            value: "trim=4956".to_owned(),
        }],
    }
}

#[test]
fn a_flagged_snapped_cut_renders_byte_identical_on_every_container() {
    let cases: [(&str, Vec<u8>, &str, &str); 5] = [
        ("WAV 16", wav(16), "wav", "TXXX:SOUNDCHECK"),
        ("WAV 24", wav(24), "wav", "TXXX:SOUNDCHECK"),
        ("AIFF 24", aiff(), "aif", "TXXX:SOUNDCHECK"),
        ("FLAC 16", flac(16), "flac", "SOUNDCHECK"),
        ("FLAC 24", flac(24), "flac", "SOUNDCHECK"),
    ];
    for (name, bytes, ext, label) in cases {
        let src = Source::new(&bytes, ext);
        let snapped = snap_head_cut(&src.path, T).expect("snaps");
        assert_eq!(
            snapped,
            T - 44,
            "{name}: the quietest frame is the window's first"
        );
        let (direct, direct_bytes) = src
            .render(&request(T, None, label), "direct")
            .expect("renders");
        assert_eq!(direct.trim_frames, snapped, "{name}: the render's own snap");
        let (flagged, flagged_bytes) = src
            .render(&request(snapped, Some(T), label), "flagged")
            .expect("renders");
        assert_eq!(flagged, direct, "{name}: the same report");
        assert_eq!(
            (flagged.trim_frames, flagged.trim_requested_frames),
            (T - 44, T),
            "{name}"
        );
        assert!(flagged_bytes == direct_bytes, "{name}: not byte identical");
        // Snapping the snapped cut again moves it another 1 ms back: a different file.
        let (again, again_bytes) = src
            .render(&request(snapped, None, label), "again")
            .expect("renders");
        assert_eq!(again.trim_frames, T - 88, "{name}");
        assert!(again_bytes != direct_bytes, "{name}");
        assert_eq!(snap_head_cut(&src.path, snapped).expect("snaps"), T - 88);
    }
}

#[test]
fn zero_and_cuts_that_leave_nothing() {
    let src = Source::new(&wav(16), "wav");
    assert_eq!(snap_head_cut(&src.path, 0).expect("no cut"), 0);
    // Within 1 ms of the start the window clamps at frame 0.
    assert_eq!(snap_head_cut(&src.path, 30).expect("snaps"), 0);
    for frames in [FRAMES as u64, FRAMES as u64 + 1] {
        assert!(matches!(
            snap_head_cut(&src.path, frames),
            Err(Error::InvalidArgument(_))
        ));
    }
    let flac = Source::new(&flac(16), "flac");
    assert!(matches!(
        snap_head_cut(&flac.path, FRAMES as u64),
        Err(Error::InvalidArgument(_))
    ));
    let text = Source::new(b"not audio at all", "wav");
    assert!(matches!(
        snap_head_cut(&text.path, T),
        Err(Error::UnsupportedFormat { .. })
    ));
    assert!(matches!(
        snap_head_cut(Path::new("/nonexistent/in.wav"), T),
        Err(Error::Io { .. })
    ));
}

#[test]
fn a_flagged_cut_outside_the_window_is_refused() {
    for (bytes, ext) in [(wav(16), "wav"), (flac(16), "flac")] {
        let src = Source::new(&bytes, ext);
        // Later than the request, or more than 44 frames before it.
        for (i, (trim, from)) in [(T + 1, T), (T - 45, T), (T, T - 1)]
            .into_iter()
            .enumerate()
        {
            let name = format!("out{i}");
            let err = src
                .render(&request(trim, Some(from), "SOUNDCHECK"), &name)
                .expect_err("refused");
            assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
            assert!(!src.dir.path().join(&name).exists(), "no output left");
        }
        // The window's ends are accepted, and a snap to the start cuts nothing.
        for (trim, from) in [(T - 44, T), (T, T), (0, 44)] {
            let label = if ext == "flac" {
                "SOUNDCHECK"
            } else {
                "TXXX:SOUNDCHECK"
            };
            let (report, _) = src
                .render(&request(trim, Some(from), label), &format!("ok{trim}"))
                .expect("renders");
            assert_eq!(
                (report.trim_frames, report.trim_requested_frames),
                (trim, from)
            );
        }
    }
}
