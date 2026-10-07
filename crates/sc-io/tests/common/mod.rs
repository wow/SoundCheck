//! Shared helpers of the transaction tests: small WAV, AIFF and FLAC files built in code (fixed
//! seeds), simulated volumes, and checks that a folder holds no temp file.
#![allow(dead_code)] // each test binary uses part of it

use std::path::{Path, PathBuf};

use sc_core::Result;
use sc_io::flac::{FrameEncoder, MARKER, StreamInfo, block_header, block_type};
use sc_io::txn::{TEMP_MARKER, Volume, VolumeProvider};

/// Deterministic 16-bit stereo samples: a ramp plus xorshift noise, within half of full scale.
pub fn samples16(frames: usize, seed: u32) -> Vec<i32> {
    let mut x = seed.max(1);
    (0..frames * 2)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let ramp = i32::try_from(i % 3000).expect("small") * 5 - 7500;
            ramp + (x.cast_signed() >> 20)
        })
        .collect()
}

fn le32(n: usize) -> [u8; 4] {
    u32::try_from(n).expect("small").to_le_bytes()
}

fn be32(n: usize) -> [u8; 4] {
    u32::try_from(n).expect("small").to_be_bytes()
}

/// A 44.1 kHz stereo 16-bit WAV with a `LIST` chunk after the audio.
pub fn wav(frames: usize, seed: u32) -> Vec<u8> {
    let data: Vec<u8> = samples16(frames, seed)
        .iter()
        .flat_map(|s| i16::try_from(*s).expect("16-bit").to_le_bytes())
        .collect();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1_u16.to_le_bytes());
    fmt.extend_from_slice(&2_u16.to_le_bytes());
    fmt.extend_from_slice(&44_100_u32.to_le_bytes());
    fmt.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    fmt.extend_from_slice(&4_u16.to_le_bytes());
    fmt.extend_from_slice(&16_u16.to_le_bytes());
    let list = b"INFOINAM\x06\x00\x00\x00track\x00".to_vec();
    let mut body = b"WAVE".to_vec();
    for (id, payload) in [(b"fmt ", &fmt), (b"data", &data), (b"LIST", &list)] {
        body.extend_from_slice(id);
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

/// A 44.1 kHz stereo 16-bit AIFF.
pub fn aiff(frames: usize, seed: u32) -> Vec<u8> {
    let data: Vec<u8> = samples16(frames, seed)
        .iter()
        .flat_map(|s| i16::try_from(*s).expect("16-bit").to_be_bytes())
        .collect();
    let mut comm = Vec::new();
    comm.extend_from_slice(&2_u16.to_be_bytes());
    comm.extend_from_slice(&be32(frames));
    comm.extend_from_slice(&16_u16.to_be_bytes());
    // 44,100 as an 80-bit IEEE 754 extended number.
    comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    let mut ssnd = vec![0_u8; 8];
    ssnd.extend_from_slice(&data);
    let mut body = b"AIFF".to_vec();
    for (id, payload) in [(b"COMM", &comm), (b"SSND", &ssnd)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&be32(payload.len()));
        body.extend_from_slice(payload);
    }
    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&be32(body.len()));
    file.extend_from_slice(&body);
    file
}

/// A 44.1 kHz stereo 16-bit FLAC with a `VORBIS_COMMENT` block, encoded by
/// [`FrameEncoder`].
pub fn flac(frames: usize, seed: u32) -> Vec<u8> {
    let mut enc = FrameEncoder::new(44_100, 2, 16, Path::new("fixture.flac")).expect("encoder");
    let mut out = Vec::new();
    enc.push(&samples16(frames, seed), &mut out)
        .expect("encode");
    let encoded = enc.finish(&mut out).expect("finish");
    let info = StreamInfo {
        min_block: 4096,
        max_block: 4096,
        min_frame: encoded.min_frame,
        max_frame: encoded.max_frame,
        sample_rate_hz: 44_100,
        channels: 2,
        bits: 16,
        total_samples: encoded.total_samples,
        md5: encoded.md5,
    };
    let mut comment = le32(4).to_vec();
    comment.extend_from_slice(b"test");
    comment.extend_from_slice(&le32(1));
    let field = b"TITLE=track";
    comment.extend_from_slice(&le32(field.len()));
    comment.extend_from_slice(field);
    let mut file = MARKER.to_vec();
    file.extend_from_slice(&block_header(false, block_type::STREAMINFO, 34));
    file.extend_from_slice(&info.to_bytes());
    let len = u32::try_from(comment.len()).expect("small");
    file.extend_from_slice(&block_header(true, block_type::VORBIS_COMMENT, len));
    file.extend_from_slice(&comment);
    file.extend_from_slice(&out);
    file
}

/// BLAKE3 of the file at `path`.
pub fn blake3_of(path: &Path) -> [u8; 32] {
    *blake3::hash(&std::fs::read(path).expect("readable")).as_bytes()
}

/// Every file under `dir` (recursively) whose name holds the temp marker.
pub fn temps_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(temps_under(&path));
        } else if entry.file_name().to_string_lossy().contains(TEMP_MARKER) {
            found.push(path);
        }
    }
    found
}

/// Asserts that no file under `dir` holds the temp marker.
pub fn assert_no_temps(dir: &Path) {
    assert_eq!(temps_under(dir), Vec::<PathBuf>::new(), "temp files left");
}

/// Every regular file under `dir` (recursively).
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// A simulated volume: every path lies on one volume rooted at `root` with `free_bytes`.
pub struct FakeVolumes {
    /// The volume's root.
    pub root: PathBuf,
    /// Bytes free.
    pub free_bytes: u64,
}

impl VolumeProvider for FakeVolumes {
    fn volume_of(&self, _path: &Path) -> Result<Volume> {
        Ok(Volume {
            root: self.root.clone(),
            name: "Fake".into(),
            id: 1,
            free_bytes: self.free_bytes,
        })
    }
}

/// A temp folder holding `music/` (the files) and `backups/` (the backup root).
pub struct Library {
    /// Keeps the folder alive.
    pub dir: tempfile::TempDir,
    /// Where the music files are.
    pub music: PathBuf,
    /// The backup root.
    pub backups: PathBuf,
}

impl Library {
    /// An empty library.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        // Resolved, so paths compare equal to the ones transactions report (macOS temp
        // folders lie behind the /var -> /private/var link).
        let base = dir.path().canonicalize().expect("temp dir resolves");
        let music = base.join("music");
        let backups = base.join("backups");
        std::fs::create_dir_all(&music).expect("music folder");
        Self {
            dir,
            music,
            backups,
        }
    }

    /// Writes `bytes` to `music/<name>`.
    pub fn add(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.music.join(name);
        std::fs::write(&path, bytes).expect("fixture written");
        path
    }
}
