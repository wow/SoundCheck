//! Unit tests of `crates/sc-io/src/render/flac.rs`: identity and gain on the samples, carried
//! bytes around and between the blocks, the report, determinism, every refusal leaving no
//! output, cancellation, and verification catching a damaged file. The fixture matrix
//! (`tests/matrix.rs`) covers the full block-by-block contract.

#![allow(clippy::cast_possible_truncation)] // test sizes and offsets are far below 2^32

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, TagEdit};

use super::write::{Expected, verify};
use super::*;
use crate::flac::test_build::{encode, file, id3v1, id3v2, samples, vorbis};
use crate::flac::{FlacLayout, StreamInfo, read_layout};
use crate::render::{BlockFate, BlockId};

struct Run {
    _dir: tempfile::TempDir,
    input: PathBuf,
    output: PathBuf,
}

fn setup(bytes: &[u8]) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("in.flac");
    std::fs::write(&input, bytes).expect("write");
    let output = dir.path().join("out.flac");
    Run {
        _dir: dir,
        input,
        output,
    }
}

fn apply(run: &Run, req: &RenderRequest) -> Result<RenderReport> {
    apply_flac(&run.input, &run.output, req, &AtomicBool::new(false))
}

fn layout_of(bytes: &[u8]) -> FlacLayout {
    read_layout(&mut Cursor::new(bytes), Path::new("x.flac")).expect("walks")
}

/// Decodes every sample of a FLAC file.
fn pcm(bytes: &[u8]) -> Vec<i32> {
    let layout = layout_of(bytes);
    let mut r = crate::flac::FlacPcm::open(Cursor::new(bytes), Path::new("x.flac"), &layout, true)
        .expect("open");
    let (mut all, mut block) = (Vec::new(), Vec::new());
    while r.next_block(&mut block).expect("decodes") > 0 {
        all.extend_from_slice(&block);
    }
    r.finish().expect("count and MD5");
    all
}

/// A 16-bit stereo source with leading and trailing tags and every kind of block.
fn tagged_source(frames: usize) -> (Vec<u8>, Vec<i32>) {
    let data = samples(frames, 2, 16, 31);
    let (enc, info) = encode(&data, 44_100, 2, 16);
    let blocks = [
        (
            4,
            vorbis("ref", &["TITLE=t", "replaygain_track_gain=-1 dB"]),
        ),
        (2, b"riffopaque".to_vec()),
        (6, vec![9; 50]),
        (77, vec![1, 2, 3]),
        (1, vec![0; 64]),
    ];
    (file(&id3v2(16), &info, &blocks, &enc, &id3v1()), data)
}

#[test]
fn identity_keeps_the_samples_and_every_carried_byte() {
    use BlockFate::{Carried, Replaced};
    let (src, data) = tagged_source(10_000);
    let run = setup(&src);
    let report = apply(&run, &RenderRequest::default()).expect("renders");
    let out = std::fs::read(&run.output).expect("output");
    assert_eq!(pcm(&out), data);
    let (a, b) = (layout_of(&src), layout_of(&out));
    assert_eq!(&out[..26], &src[..26], "the leading ID3v2 tag");
    assert_eq!(
        out[out.len() - 128..],
        src[src.len() - 128..],
        "the ID3v1 tag"
    );
    for (x, y) in a.blocks.iter().zip(&b.blocks).skip(1) {
        let bytes =
            |f: &[u8], r: &std::ops::Range<u64>| f[r.start as usize..r.end as usize].to_vec();
        assert_eq!(
            bytes(&src, &x.payload),
            bytes(&out, &y.payload),
            "{}",
            x.name()
        );
        assert_eq!(x.last, y.last);
    }
    assert_eq!(b.streaminfo.total_samples, 10_000);
    assert_eq!(
        (b.streaminfo.min_block, b.streaminfo.max_block),
        (4096, 4096)
    );
    let fates: Vec<BlockFate> = report.blocks.iter().map(|r| r.fate).collect();
    assert_eq!(
        fates,
        [Replaced, Carried, Carried, Carried, Carried, Replaced]
    );
    assert_eq!(report.blocks[4].id, BlockId::FlacBlock(77));
    assert!(report.exact && !report.dithered);
    assert_eq!((report.leading_bytes, report.trailing_bytes), (26, 128));
    assert_eq!(report.output_bytes, out.len() as u64);
    assert_eq!(
        (report.frames_in, report.frames_out, report.bits_out),
        (10_000, 10_000, 16)
    );
    assert!(!report.tags_added && report.tags_not_added.is_none());
}

#[test]
fn gain_trim_and_tags_on_a_24_bit_source() {
    let data = samples(9000, 2, 24, 33);
    let (enc, info) = encode(&data, 48_000, 2, 24);
    let src = file(
        &[],
        &info,
        &[(4, vorbis("v", &["REPLAYGAIN_TRACK_GAIN=0 dB"]))],
        &enc,
        &[],
    );
    let run = setup(&src);
    let req = RenderRequest {
        gain_db: -3.2,
        trim_frames: 441,
        tag_edits: vec![
            TagEdit {
                label: "replaygain_track_gain".into(),
                value: "-6.20 dB".into(),
            },
            TagEdit {
                label: "SOUNDCHECK".into(),
                value: "{}".into(),
            },
        ],
        ..RenderRequest::default()
    };
    let report = apply(&run, &req).expect("renders");
    let out = std::fs::read(&run.output).expect("output");
    let got = pcm(&out);
    let g = sc_dsp::db_to_linear(-3.2);
    let want: Vec<i32> = data[441 * 2..]
        .iter()
        .map(|x| {
            // Rounded half to even, as the requantiser does at 24 bits.
            #[allow(clippy::cast_possible_truncation)]
            let y = (f64::from(*x) * g).round_ties_even() as i32;
            y
        })
        .collect();
    assert_eq!(got, want);
    assert_eq!((report.frames_out, report.bits_out), (8559, 24));
    assert!(report.tags_added && !report.exact && !report.dithered);
    let summary = report.vorbis_edit.expect("edited");
    assert_eq!((summary.replaced, summary.appended), (1, 1));
    let l = layout_of(&out);
    let comment = &l.blocks[1];
    let p = &out[comment.payload.start as usize..comment.payload.end as usize];
    assert_eq!(
        p,
        vorbis("v", &["REPLAYGAIN_TRACK_GAIN=-6.20 dB", "SOUNDCHECK={}"])
    );
}

#[test]
fn depth_follows_the_request_or_the_source() {
    let data = samples(5000, 1, 20, 35);
    let (enc, info) = encode(&data, 44_100, 1, 20);
    let run = setup(&file(&[], &info, &[], &enc, &[]));
    let report = apply(&run, &RenderRequest::default()).expect("renders");
    let out = std::fs::read(&run.output).expect("output");
    assert_eq!(report.bits_out, 24);
    assert!(report.exact);
    let shifted: Vec<i32> = data.iter().map(|x| x << 4).collect();
    assert_eq!(pcm(&out), shifted);
    std::fs::remove_file(&run.output).expect("rm");
    let req = RenderRequest {
        bits: Some(16),
        ..RenderRequest::default()
    };
    let report = apply(&run, &req).expect("renders");
    assert_eq!(report.bits_out, 16);
    assert!(report.dithered);
}

#[test]
fn the_same_request_gives_the_same_bytes() {
    let (src, _) = tagged_source(7000);
    let run = setup(&src);
    let req = RenderRequest {
        gain_db: -1.5,
        bits: Some(16),
        ..RenderRequest::default()
    };
    apply(&run, &req).expect("renders");
    let first = std::fs::read(&run.output).expect("output");
    std::fs::remove_file(&run.output).expect("rm");
    apply(&run, &req).expect("renders");
    assert_eq!(std::fs::read(&run.output).expect("output"), first);
}

fn refused(src: &[u8], req: &RenderRequest) -> Error {
    let run = setup(src);
    let err = apply(&run, req).expect_err("refused");
    assert!(!run.output.exists(), "a refusal left an output: {err}");
    err
}

#[test]
fn refusals_leave_no_output() {
    let mono = samples(5000, 1, 16, 37);
    let (enc, info) = encode(&mono, 44_100, 1, 16);
    let good = file(&[], &info, &[], &enc, &[]);
    let trim = RenderRequest {
        trim_frames: 5000,
        ..RenderRequest::default()
    };
    assert!(matches!(refused(&good, &trim), Error::InvalidArgument(_)));
    let bad_tag = RenderRequest {
        tag_edits: vec![TagEdit {
            label: "A=B".into(),
            value: "x".into(),
        }],
        ..RenderRequest::default()
    };
    assert!(matches!(
        refused(&good, &bad_tag),
        Error::InvalidArgument(_)
    ));
    let boost = RenderRequest {
        gain_db: 7.0,
        ..RenderRequest::default()
    };
    assert!(matches!(refused(&good, &boost), Error::WouldClip { .. }));
    let three = samples(5000, 3, 16, 39);
    let (enc3, info3) = encode(&three, 44_100, 3, 16);
    let err = refused(
        &file(&[], &info3, &[], &enc3, &[]),
        &RenderRequest::default(),
    );
    assert!(matches!(
        err,
        Error::UnsupportedChannels { channels: 3, .. }
    ));
    let (enc96, info96) = encode(&mono, 96_000, 1, 16);
    let err = refused(
        &file(&[], &info96, &[], &enc96, &[]),
        &RenderRequest::default(),
    );
    assert!(matches!(err, Error::NotDjSafe { .. }));
    let mut damaged = good.clone();
    let at = damaged.len() - 200;
    damaged[at] ^= 0x20;
    assert!(matches!(
        refused(&damaged, &RenderRequest::default()),
        Error::Corrupt { .. }
    ));
    let wrong_md5 = file(
        &[],
        &StreamInfo {
            md5: [1; 16],
            ..info
        },
        &[],
        &enc,
        &[],
    );
    assert!(matches!(
        refused(&wrong_md5, &RenderRequest::default()),
        Error::Corrupt { .. }
    ));
}

#[test]
fn an_existing_output_and_cancellation() {
    let (src, _) = tagged_source(5000);
    let run = setup(&src);
    std::fs::write(&run.output, b"keep").expect("write");
    let err = apply(&run, &RenderRequest::default()).expect_err("exists");
    assert!(matches!(&err, Error::Io { path, .. } if *path == run.output));
    assert_eq!(std::fs::read(&run.output).expect("kept"), b"keep");
    std::fs::remove_file(&run.output).expect("rm");
    let err = apply_flac(
        &run.input,
        &run.output,
        &RenderRequest::default(),
        &AtomicBool::new(true),
    )
    .expect_err("cancelled");
    assert!(matches!(err, Error::Cancelled));
    assert!(!run.output.exists());
}

#[test]
fn an_unknown_total_is_counted() {
    let data = samples(6000, 2, 16, 41);
    let (enc, info) = encode(&data, 44_100, 2, 16);
    let unknown = StreamInfo {
        total_samples: 0,
        ..info
    };
    let run = setup(&file(&[], &unknown, &[], &enc, &id3v1()));
    let report = apply(&run, &RenderRequest::default()).expect("renders");
    assert_eq!(report.frames_out, 6000);
    let out = std::fs::read(&run.output).expect("output");
    assert_eq!(layout_of(&out).streaminfo.total_samples, 6000);
    assert_eq!(pcm(&out), data);
}

#[test]
fn verification_catches_damage_on_disk() {
    let (src, _) = tagged_source(9000);
    let run = setup(&src);
    let report = apply(&run, &RenderRequest::default()).expect("renders");
    let out = std::fs::read(&run.output).expect("output");
    let l = layout_of(&out);
    let want = Expected {
        block_types: l.blocks.iter().map(|b| b.block_type).collect(),
        frames_start: l.frames_start,
        info: l.streaminfo,
        frames_bytes: l.file_len - 128 - l.frames_start,
        pcm_hash: report.pcm_hash,
    };
    let no = AtomicBool::new(false);
    verify(&run.output, &want, &no).expect("the output verifies");
    let wrong_hash = Expected {
        pcm_hash: [0; 32],
        ..want.clone()
    };
    assert!(matches!(
        verify(&run.output, &wrong_hash, &no),
        Err(Error::Corrupt { path, .. }) if path == run.output
    ));
    let mut damaged = out.clone();
    let mid = usize::try_from(l.frames_start).expect("small") + 5000;
    damaged[mid] ^= 0x01;
    std::fs::write(&run.output, &damaged).expect("write");
    assert!(matches!(
        verify(&run.output, &want, &no),
        Err(Error::Corrupt { .. })
    ));
    let mut md5 = out;
    md5[usize::try_from(l.blocks[0].payload.start).expect("small") + 20] ^= 1;
    std::fs::write(&run.output, &md5).expect("write");
    let new_info = layout_of(&md5).streaminfo;
    let same_info = Expected {
        info: new_info,
        ..want
    };
    assert!(matches!(
        verify(&run.output, &same_info, &no),
        Err(Error::Corrupt { .. })
    ));
}
