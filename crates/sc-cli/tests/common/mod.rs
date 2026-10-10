//! Shared helpers of the `apply`, `undo`, `journal` and `recover` tests: small WAV files built
//! in code, a temp library with its own backup root (`SC_BACKUP_ROOT`), and running `sc-cli`.
#![allow(dead_code)] // each test binary uses part of it

use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

fn le32(n: usize) -> [u8; 4] {
    u32::try_from(n).expect("small").to_le_bytes()
}

/// The samples of every fixture: 22,050 stereo frames of a ramp peaking near -12 dBFS.
fn samples() -> impl Iterator<Item = i16> {
    (0..44_100_i32).map(|i| i16::try_from((i % 2000) * 8 - 8000).expect("16-bit"))
}

/// An ID3v2.3 tag with one `TIT2` frame and 64 bytes of padding.
fn id3_tag() -> Vec<u8> {
    let mut frame = b"TIT2".to_vec();
    frame.extend_from_slice(&6_u32.to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0]);
    frame.extend_from_slice(b"Title");
    let size = frame.len() + 64;
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    tag.extend_from_slice(&[0, 0, 0, u8::try_from(size).expect("under 128")]);
    tag.extend_from_slice(&frame);
    tag.resize(tag.len() + 64, 0);
    tag
}

/// A 0.5 s 44.1 kHz stereo 16-bit WAV (see [`wav_at`]).
pub fn wav_bytes(id3: bool) -> Vec<u8> {
    wav_at(44_100, id3)
}

/// A stereo 16-bit WAV of 22,050 frames at `rate` Hz, with an ID3v2.3 `id3 ` chunk after the
/// audio when `id3` is set.
pub fn wav_at(rate: u32, id3: bool) -> Vec<u8> {
    let data: Vec<u8> = samples().flat_map(i16::to_le_bytes).collect();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1_u16.to_le_bytes());
    fmt.extend_from_slice(&2_u16.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * 4).to_le_bytes());
    fmt.extend_from_slice(&4_u16.to_le_bytes());
    fmt.extend_from_slice(&16_u16.to_le_bytes());
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"fmt ", fmt), (b"data", data)];
    if id3 {
        chunks.push((b"id3 ", id3_tag()));
    }
    let mut body = b"WAVE".to_vec();
    for (id, payload) in &chunks {
        body.extend_from_slice(*id);
        body.extend_from_slice(&le32(payload.len()));
        body.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&le32(body.len()));
    file.extend_from_slice(&body);
    file
}

/// A 0.5 s 44.1 kHz stereo 16-bit AIFF with an ID3v2.3 `ID3 ` chunk after the audio.
pub fn aiff_with_id3() -> Vec<u8> {
    let be32 = |n: usize| u32::try_from(n).expect("small").to_be_bytes();
    let data: Vec<u8> = samples().flat_map(i16::to_be_bytes).collect();
    let mut comm = Vec::new();
    comm.extend_from_slice(&2_u16.to_be_bytes());
    comm.extend_from_slice(&be32(22_050));
    comm.extend_from_slice(&16_u16.to_be_bytes());
    // 44,100 as an 80-bit IEEE 754 extended number.
    comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    let mut ssnd = vec![0_u8; 8];
    ssnd.extend_from_slice(&data);
    let mut body = b"AIFF".to_vec();
    for (id, payload) in [(b"COMM", comm), (b"SSND", ssnd), (b"ID3 ", id3_tag())] {
        body.extend_from_slice(id);
        body.extend_from_slice(&be32(payload.len()));
        body.extend_from_slice(&payload);
        if payload.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&be32(body.len()));
    file.extend_from_slice(&body);
    file
}

/// A temp folder holding `music/` and the backup root `backups/`.
pub struct Library {
    _dir: tempfile::TempDir,
    /// The temp folder, resolved.
    pub base: PathBuf,
    /// Where the test files are.
    pub music: PathBuf,
    /// The backup root (not created).
    pub backups: PathBuf,
}

impl Library {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let base = dir.path().canonicalize().expect("resolves");
        let music = base.join("music");
        std::fs::create_dir_all(&music).expect("music");
        Self {
            _dir: dir,
            backups: base.join("backups"),
            music,
            base,
        }
    }

    /// Writes `bytes` as `music/<name>`.
    pub fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.music.join(name);
        std::fs::write(&path, bytes).expect("write");
        path
    }

    /// Runs `sc-cli` with the backup root in `SC_BACKUP_ROOT`.
    pub fn run(&self, args: &[&str]) -> Run {
        self.run_in(&self.base, args)
    }

    /// Runs `sc-cli` in the folder `cwd`, with the backup root in `SC_BACKUP_ROOT`.
    pub fn run_in(&self, cwd: &Path, args: &[&str]) -> Run {
        let output = Command::cargo_bin("sc-cli")
            .expect("binary")
            .env("SC_BACKUP_ROOT", &self.backups)
            .env("SC_CACHE_DIR", self.base.join("cache"))
            .env("SC_EDITS_DIR", self.base.join("edits"))
            .env("SC_EXPORTS_ROOT", self.base.join("exports"))
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("run");
        Run::from(output)
    }
}

/// What a run printed and how it ended.
pub struct Run {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl From<Output> for Run {
    fn from(o: Output) -> Self {
        Self {
            ok: o.status.success(),
            code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }
}

impl Run {
    /// Both streams, for assertion messages.
    pub fn text(&self) -> String {
        format!("stdout:\n{}\nstderr:\n{}", self.stdout, self.stderr)
    }

    /// Stdout as consecutive JSON documents.
    pub fn docs(&self) -> Vec<serde_json::Value> {
        serde_json::Deserializer::from_str(&self.stdout)
            .into_iter::<serde_json::Value>()
            .map(|d| d.expect("JSON document"))
            .collect()
    }
}

/// `path` as a command-line argument.
pub fn arg(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
}

/// Every file under `dir`, recursively.
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(list) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in list.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}
