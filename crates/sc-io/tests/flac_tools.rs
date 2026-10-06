//! FLAC renders of files made by libFLAC itself (the reference `flac` encoder), checked by
//! libFLAC's own `flac -t`. Opt-in: `SC_FLAC_TOOLS=1` and the `flac` tool installed (CI installs
//! it); the test passes as a no-op otherwise.
//!
//! The sources are WAV files written here and encoded by `flac` with settings our own encoder
//! never uses: fixed block sizes of 1152 and 4608 samples, a last frame shorter than 16 samples,
//! wasted bits (samples whose low bits are all zero), 8-, 16- and 24-bit, mono and stereo, and
//! libFLAC's default metadata (SEEKTABLE, `VORBIS_COMMENT`, PADDING). Each is rendered at 0 dB
//! (the decoded output must equal the source samples shifted to the output depth) and at
//! -3.2 dB, and every output must pass `flac -t -w` (warnings are errors). Variable block sizes
//! are not covered: the `flac` command line has no option to produce them.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use sc_core::RenderRequest;
use sc_io::flac::{FlacPcm, read_layout};
use sc_io::render::apply_flac;

/// One source: its WAV parameters, how to fill it and the block size to encode it with.
struct Case {
    name: &'static str,
    channels: u16,
    bits: u16,
    frames: usize,
    /// Low bits left zero in every sample.
    wasted: u32,
    blocksize: u32,
}

const CASES: [Case; 5] = [
    Case {
        name: "s16-1152-short-tail",
        channels: 2,
        bits: 16,
        frames: 1152 * 7 + 10,
        wasted: 0,
        blocksize: 1152,
    },
    Case {
        name: "m24-4608-short-tail",
        channels: 1,
        bits: 24,
        frames: 4608 * 3 + 5,
        wasted: 0,
        blocksize: 4608,
    },
    Case {
        name: "s8-4096",
        channels: 2,
        bits: 8,
        frames: 10_000,
        wasted: 0,
        blocksize: 4096,
    },
    Case {
        name: "s16-wasted-4",
        channels: 2,
        bits: 16,
        frames: 9000,
        wasted: 4,
        blocksize: 4096,
    },
    Case {
        name: "m24-wasted-8",
        channels: 1,
        bits: 24,
        frames: 9000,
        wasted: 8,
        blocksize: 2048,
    },
];

/// Deterministic samples at about a quarter of full scale with `wasted` zero low bits.
fn samples(case: &Case) -> Vec<i32> {
    let mut x = 0x2545_F491_u32;
    let shift = 34 - u32::from(case.bits);
    (0..case.frames * usize::from(case.channels))
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x.cast_signed() >> shift) >> case.wasted << case.wasted
        })
        .collect()
}

fn write_wav(path: &Path, case: &Case, data: &[i32]) {
    let spec = hound::WavSpec {
        channels: case.channels,
        sample_rate: 44_100,
        bits_per_sample: case.bits,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("wav");
    for s in data {
        if case.bits == 8 {
            w.write_sample(i8::try_from(*s).expect("8-bit"))
                .expect("write");
        } else {
            w.write_sample(*s).expect("write");
        }
    }
    w.finalize().expect("wav");
}

fn run(cmd: &mut Command, what: &str) {
    let out = cmd.output().expect("run flac");
    assert!(
        out.status.success(),
        "{what}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Every sample of a FLAC file.
fn decode(path: &Path) -> Vec<i32> {
    let mut file = std::fs::File::open(path).expect("open");
    let layout = read_layout(&mut file, path).expect("walks");
    let mut pcm = FlacPcm::open(file, path, &layout, true).expect("decoder");
    let (mut all, mut block) = (Vec::new(), Vec::new());
    while pcm.next_block(&mut block).expect("decodes") > 0 {
        all.extend_from_slice(&block);
    }
    pcm.finish().expect("count and MD5");
    all
}

fn tools_enabled() -> bool {
    if std::env::var("SC_FLAC_TOOLS").as_deref() != Ok("1") {
        eprintln!("flac_tools: skipped; set SC_FLAC_TOOLS=1 to run");
        return false;
    }
    if Command::new("flac").arg("--version").output().is_err() {
        eprintln!("flac_tools: skipped; the flac tool is not installed");
        return false;
    }
    true
}

#[test]
fn libflac_sources_render_and_pass_flac_t() {
    if !tools_enabled() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    for case in &CASES {
        let data = samples(case);
        let wav = dir.path().join(format!("{}.wav", case.name));
        let src = dir.path().join(format!("{}.flac", case.name));
        write_wav(&wav, case, &data);
        run(
            Command::new("flac")
                .args(["-s", "-f", &format!("--blocksize={}", case.blocksize), "-o"])
                .arg(&src)
                .arg(&wav),
            case.name,
        );
        assert_eq!(
            decode(&src),
            data,
            "{}: our decoder reads libFLAC",
            case.name
        );
        for gain_db in [0.0, -3.2] {
            let out = dir.path().join(format!("{}-{gain_db}.flac", case.name));
            let req = RenderRequest {
                gain_db,
                ..RenderRequest::default()
            };
            let report = apply_flac(&src, &out, &req, &AtomicBool::new(false))
                .unwrap_or_else(|e| panic!("{} {gain_db} dB: {e}", case.name));
            assert_eq!(report.frames_out, case.frames as u64, "{}", case.name);
            let bits_out = if case.bits > 16 { 24 } else { 16 };
            assert_eq!(report.bits_out, bits_out, "{}", case.name);
            run(
                Command::new("flac").args(["-t", "-s", "-w"]).arg(&out),
                &format!("{} {gain_db} dB: flac -t", case.name),
            );
            if gain_db == 0.0 {
                let shifted: Vec<i32> = data.iter().map(|s| s << (bits_out - case.bits)).collect();
                assert_eq!(decode(&out), shifted, "{}: identity", case.name);
                assert!(report.exact, "{}", case.name);
            }
        }
    }
}
