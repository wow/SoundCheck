//! Shared test helpers: synthetic WAV files and settings.

#![allow(dead_code)] // each test binary uses a subset

use std::path::{Path, PathBuf};

use sc_core::analysis::{AnalysisSettings, Model};
use sc_core::{AudioSpec, Bpm, testsig};

/// Writes a 16-bit stereo 44.1 kHz WAV of a 1 kHz tone (`seconds` long, amplitude `amp`).
pub fn tone_wav(dir: &Path, name: &str, seconds: f64, amp: f32) -> PathBuf {
    let path = dir.join(name);
    let buf = testsig::sine(AudioSpec::CD, 1000.0, amp, seconds);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

/// A file with a `.wav` name whose bytes are not a WAV.
pub fn corrupt_wav(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"RIFF\x04\x00\x00\x00WAVEnot a real wave file").expect("write");
    path
}

/// Loudness-only settings: no model needed, so these tests run on every machine.
pub fn loudness_only() -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: false,
        model: Model::Small,
    }
}
