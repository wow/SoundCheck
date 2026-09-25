//! Beat and downbeat tracking with "Beat This!" (Foscarin, Schlüter and Widmer, ISMIR 2024)
//! through the `beat-this` crate, a Rust port that runs the model's ONNX graphs on `rten`.
//!
//! Input is 22 050 Hz mono. Output is the model's beat and downbeat times plus its raw 50 fps
//! activations, which the meter estimator and the grid solver treat as evidence, never as the
//! grid: the beats are quantised to 20 ms frames and the downbeats are unreliable on odd meters.
//!
//! The model files (`mel_spectrogram.onnx`, `beat_this_small.onnx`) are not compiled in. They
//! are looked up, in order, in `SC_MODEL_DIR`, next to the executable (`models/`, and
//! `../Resources/models` inside an app bundle), `models/` under the working directory (a
//! development checkout after `scripts/fetch-models.sh`), and the user's application-support
//! directory.

use std::path::{Path, PathBuf};

use beat_this::{BeatThis, RtenRuntime, Runtime};
use sc_core::{Error, Result, SampleIndex};

/// Sample rate the model expects.
pub const MODEL_SAMPLE_RATE: u32 = 22_050;

/// Frames per second of the model's activations (20 ms per frame).
pub const MODEL_FPS: f64 = 50.0;

/// Environment variable naming the model directory.
pub const MODEL_DIR_ENV: &str = "SC_MODEL_DIR";

/// The mel-spectrogram front end.
pub const MEL_MODEL_FILE: &str = "mel_spectrogram.onnx";

/// The bundled small model (about 10 MB).
pub const BEAT_MODEL_FILE: &str = "beat_this_small.onnx";

/// The full model, a developer option (`SC_MODEL=full`).
pub const BEAT_MODEL_FULL_FILE: &str = "beat_this.onnx";

/// What the model says about a track.
#[derive(Debug, Clone, PartialEq)]
pub struct RawBeats {
    /// Beat times in seconds, ascending.
    pub beats_s: Vec<f32>,
    /// Downbeat times in seconds, ascending, each on a beat.
    pub downbeats_s: Vec<f32>,
    /// Beat activation per 20 ms frame.
    pub beat_logits: Vec<f32>,
    /// Downbeat activation per 20 ms frame.
    pub downbeat_logits: Vec<f32>,
}

impl RawBeats {
    /// Beat positions as sample indices at `sample_rate`.
    #[must_use]
    pub fn beats_at(&self, sample_rate: u32) -> Vec<SampleIndex> {
        to_samples(&self.beats_s, sample_rate)
    }

    /// Downbeat positions as sample indices at `sample_rate`.
    #[must_use]
    pub fn downbeats_at(&self, sample_rate: u32) -> Vec<SampleIndex> {
        to_samples(&self.downbeats_s, sample_rate)
    }
}

fn to_samples(seconds: &[f32], sample_rate: u32) -> Vec<SampleIndex> {
    seconds
        .iter()
        .map(|&s| sc_core::Seconds(f64::from(s)).to_sample_index(sample_rate))
        .collect()
}

/// Directories searched for the model files, in order.
#[must_use]
pub fn model_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = std::env::var_os(MODEL_DIR_ENV) {
        paths.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        paths.push(exe_dir.join("models"));
        paths.push(exe_dir.join("../Resources/models"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("models"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            PathBuf::from(home).join("Library/Application Support/app.soundcheck.desktop/models"),
        );
    }
    paths
}

/// The first search path that holds both model files.
///
/// # Errors
/// [`Error::ModelUnavailable`] listing every directory tried.
pub fn find_model_dir() -> Result<PathBuf> {
    let searched = model_search_paths();
    searched
        .iter()
        .find(|dir| dir.join(MEL_MODEL_FILE).is_file() && dir.join(BEAT_MODEL_FILE).is_file())
        .cloned()
        .ok_or(Error::ModelUnavailable { searched })
}

/// A loaded model; one per worker, about 150-250 MB resident.
pub struct BeatTracker {
    inner: BeatThis<<RtenRuntime as Runtime>::Model>,
    model_file: String,
}

impl std::fmt::Debug for BeatTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeatTracker")
            .field("model_file", &self.model_file)
            .finish_non_exhaustive()
    }
}

impl BeatTracker {
    /// Loads the small model from `dir`.
    ///
    /// # Errors
    /// [`Error::ModelUnavailable`] when a file is missing, [`Error::Internal`] when a file
    /// cannot be parsed as a model.
    pub fn load(dir: &Path) -> Result<Self> {
        Self::load_named(dir, BEAT_MODEL_FILE)
    }

    /// Loads the model file `beat_model` (small or full) from `dir`.
    ///
    /// # Errors
    /// As for [`BeatTracker::load`].
    pub fn load_named(dir: &Path, beat_model: &str) -> Result<Self> {
        let mel = dir.join(MEL_MODEL_FILE);
        let beat = dir.join(beat_model);
        if !mel.is_file() || !beat.is_file() {
            return Err(Error::ModelUnavailable {
                searched: vec![dir.to_path_buf()],
            });
        }
        let inner = BeatThis::new(&RtenRuntime, &mel, &beat).map_err(|e| {
            Error::Internal(format!(
                "loading beat-tracking model from {}: {e:#}",
                dir.display()
            ))
        })?;
        tracing::debug!(dir = %dir.display(), model = beat_model, "beat-tracking model loaded");
        Ok(Self {
            inner,
            model_file: beat_model.to_owned(),
        })
    }

    /// Loads the small model from the first directory in [`model_search_paths`] that has it.
    ///
    /// # Errors
    /// As for [`find_model_dir`] and [`BeatTracker::load`].
    pub fn load_default() -> Result<Self> {
        Self::load(&find_model_dir()?)
    }

    /// The beat-model file in use.
    #[must_use]
    pub fn model_file(&self) -> &str {
        &self.model_file
    }

    /// Tracks a 22 050 Hz mono signal.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for an empty signal, [`Error::Internal`] when inference fails.
    pub fn track(&mut self, mono_22050: &[f32]) -> Result<RawBeats> {
        if mono_22050.is_empty() {
            return Err(Error::InvalidArgument("no audio to track".into()));
        }
        let analysis = self
            .inner
            .analyze_audio(mono_22050, MODEL_SAMPLE_RATE)
            .map_err(|e| Error::Internal(format!("beat tracking: {e:#}")))?;
        Ok(RawBeats {
            beats_s: analysis.beats,
            downbeats_s: analysis.downbeats,
            beat_logits: analysis.beat_logits,
            downbeat_logits: analysis.downbeat_logits,
        })
    }
}
