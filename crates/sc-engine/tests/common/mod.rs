//! Shared test helpers: synthetic WAV files and settings.

#![allow(dead_code)] // each test binary uses a subset

pub mod export;

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

/// `lead` seconds of silence, then a 1 kHz click on every beat at `bpm` for `beats` beats (the
/// first of every four louder), 16-bit stereo 44.1 kHz.
pub fn click_wav(dir: &Path, bpm: f64, beats: u32, lead: f64) -> PathBuf {
    let path = dir.join(format!("click-{bpm}.wav"));
    let clicks = testsig::click_track(AudioSpec::CD, Bpm(bpm), beats, 15.0);
    let lead_samples = 2 * testsig::frames_for(AudioSpec::CD, lead);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for _ in 0..lead_samples {
        w.write_sample(0_i16).expect("write");
    }
    for s in &clicks.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * 0.5 * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

/// Whether the beat-tracking model is installed; prints why a test is skipped when not.
pub fn have_models() -> bool {
    let ok = sc_analysis::beats::find_model_dir().is_ok();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok
}

/// A file with a `.wav` name whose bytes are not a WAV.
pub fn corrupt_wav(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"RIFF\x04\x00\x00\x00WAVEnot a real wave file").expect("write");
    path
}

/// Full analysis settings with the grid (needs the model files).
pub fn with_grid() -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: true,
        model: Model::Small,
    }
}

/// Loudness-only settings: no model needed, so these tests run on every machine.
pub fn loudness_only() -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: false,
        model: Model::Small,
    }
}
