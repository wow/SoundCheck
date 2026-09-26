//! Beat and downbeat tracking with "Beat This!" (Foscarin, Schlüter and Widmer, ISMIR 2024)
//! through the `beat-this` crate, a Rust port that runs the model's ONNX graphs on `rten`.
//!
//! Input is 22 050 Hz mono. Output is the model's beat and downbeat times plus its raw 50 fps
//! activations, which the meter estimator and the grid solver treat as evidence, never as the
//! grid: the beats are quantised to 20 ms frames and the downbeats are unreliable on odd meters.
//!
//! The model files (`mel_spectrogram.onnx`, `beat_this_small.onnx`) are not compiled in. They
//! are looked up in `SC_MODEL_DIR` alone when it is set, otherwise, in order, next to the
//! executable (`models/`, and
//! `../Resources/models` inside an app bundle), `models/` under the working directory (a
//! development checkout after `scripts/fetch-models.sh`), in debug builds the checkout's own
//! `models/`, and the user's application-support directory.
//!
//! Inference runs the mel front end once and the beat model once per 30 s chunk. A tracker can
//! be tied to a cancel flag, checked before every model run, and [`BeatTracker::track_with`]
//! reports the fraction of chunks done, so a long track can be stopped and followed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use beat_this::{BeatThis, Model, RtenRuntime, Runtime, Tensor};
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

/// Samples per activation frame at [`MODEL_SAMPLE_RATE`] (50 fps).
const SAMPLES_PER_FRAME: usize = 441;

/// Step between beat-model chunks in activation frames, as `beat-this` cuts them: 1500-frame
/// (30 s) chunks with 6 frames discarded at each edge.
const CHUNK_STEP: usize = 1500 - 2 * 6;

/// Receives the fraction of beat-model chunks done, `0.0..=1.0`.
pub type ChunkProgress = Box<dyn FnMut(f32) + Send>;

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

/// Directories searched for the model files, in order. `SC_MODEL_DIR`, when set, is the only
/// one: an explicit choice is never quietly replaced by another copy.
#[must_use]
pub fn model_search_paths() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os(MODEL_DIR_ENV) {
        return vec![PathBuf::from(dir)];
    }
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        paths.push(exe_dir.join("models"));
        paths.push(exe_dir.join("../Resources/models"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("models"));
    }
    // Debug builds also look in the checkout's `models/` (after `scripts/fetch-models.sh`), so the
    // app started with `pnpm tauri dev` from any directory finds them. Release builds never do.
    #[cfg(debug_assertions)]
    paths.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models"));
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

/// What the model runners share with the tracker: the cancel flag, and the progress sink of
/// the track in flight with its expected chunk count and the chunks done so far.
#[derive(Default)]
struct Hooks {
    cancel: Arc<AtomicBool>,
    progress: Mutex<Option<(ChunkProgress, usize, usize)>>,
}

/// A model runner that checks the cancel flag before each run; the beat model's runner also
/// reports progress after each chunk.
struct HookedModel {
    inner: <RtenRuntime as Runtime>::Model,
    hooks: Arc<Hooks>,
    counts_chunks: bool,
}

/// The error a hooked runner returns once the flag is set; [`BeatTracker::track`] maps it to
/// [`Error::Cancelled`].
#[derive(Debug)]
struct CancelledRun;

impl std::fmt::Display for CancelledRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for CancelledRun {}

impl Model for HookedModel {
    fn run(&mut self, inputs: &[(&str, &Tensor)]) -> anyhow::Result<HashMap<String, Tensor>> {
        if self.hooks.cancel.load(Ordering::Relaxed) {
            return Err(CancelledRun.into());
        }
        let outputs = self.inner.run(inputs)?;
        if self.counts_chunks {
            let mut slot = self
                .hooks
                .progress
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some((sink, expected, done)) = slot.as_mut() {
                *done += 1;
                // Chunk counts are small; the fraction is display precision.
                #[allow(clippy::cast_precision_loss)]
                let fraction = (*done as f32 / (*expected).max(1) as f32).min(1.0);
                sink(fraction);
            }
        }
        Ok(outputs)
    }
}

/// Beat-model runs `beat-this` makes for `samples` of 22 050 Hz audio: one chunk start every
/// [`CHUNK_STEP`] frames of the `samples / 441 + 1` activation frames.
fn expected_chunks(samples: usize) -> usize {
    (samples / SAMPLES_PER_FRAME + 1)
        .div_ceil(CHUNK_STEP)
        .max(1)
}

/// A loaded model; one per worker, about 150-250 MB resident.
pub struct BeatTracker {
    inner: BeatThis<HookedModel>,
    hooks: Arc<Hooks>,
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
        Self::load_with_cancel(dir, beat_model, Arc::default())
    }

    /// As [`BeatTracker::load_named`], stopping any track in flight at its next model run once
    /// `cancel` is set.
    ///
    /// # Errors
    /// As for [`BeatTracker::load`].
    pub fn load_with_cancel(dir: &Path, beat_model: &str, cancel: Arc<AtomicBool>) -> Result<Self> {
        let mel = dir.join(MEL_MODEL_FILE);
        let beat = dir.join(beat_model);
        if !mel.is_file() || !beat.is_file() {
            return Err(Error::ModelUnavailable {
                searched: vec![dir.to_path_buf()],
            });
        }
        let load = |path: &Path| {
            RtenRuntime.load_model(path).map_err(|e| {
                Error::Internal(format!(
                    "loading beat-tracking model {}: {e:#}",
                    path.display()
                ))
            })
        };
        let hooks = Arc::new(Hooks {
            cancel,
            progress: Mutex::new(None),
        });
        let inner = BeatThis::from_models(
            HookedModel {
                inner: load(&mel)?,
                hooks: Arc::clone(&hooks),
                counts_chunks: false,
            },
            HookedModel {
                inner: load(&beat)?,
                hooks: Arc::clone(&hooks),
                counts_chunks: true,
            },
        );
        tracing::debug!(dir = %dir.display(), model = beat_model, "beat-tracking model loaded");
        Ok(Self {
            inner,
            hooks,
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
    /// [`Error::InvalidArgument`] for an empty signal, [`Error::Cancelled`] once the cancel flag
    /// given at load is set, [`Error::Internal`] when inference fails.
    pub fn track(&mut self, mono_22050: &[f32]) -> Result<RawBeats> {
        self.run(mono_22050)
    }

    /// As [`BeatTracker::track`], calling `progress` with the fraction of beat-model chunks done
    /// after each chunk (about once per 30 s of audio).
    ///
    /// # Errors
    /// As for [`BeatTracker::track`].
    pub fn track_with(&mut self, mono_22050: &[f32], progress: ChunkProgress) -> Result<RawBeats> {
        *self.progress_slot() = Some((progress, expected_chunks(mono_22050.len()), 0));
        let result = self.run(mono_22050);
        *self.progress_slot() = None;
        result
    }

    fn progress_slot(&self) -> std::sync::MutexGuard<'_, Option<(ChunkProgress, usize, usize)>> {
        self.hooks
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&mut self, mono_22050: &[f32]) -> Result<RawBeats> {
        if mono_22050.is_empty() {
            return Err(Error::InvalidArgument("no audio to track".into()));
        }
        let analysis = self
            .inner
            .analyze_audio(mono_22050, MODEL_SAMPLE_RATE)
            .map_err(|e| {
                if e.is::<CancelledRun>() {
                    Error::Cancelled
                } else {
                    Error::Internal(format!("beat tracking: {e:#}"))
                }
            })?;
        Ok(RawBeats {
            beats_s: analysis.beats,
            downbeats_s: analysis.downbeats,
            beat_logits: analysis.beat_logits,
            downbeat_logits: analysis.downbeat_logits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_chunks_follow_beat_this_chunking() {
        // 1488 activation frames per step; frames = samples / 441 + 1.
        assert_eq!(expected_chunks(0), 1);
        assert_eq!(expected_chunks(441 * 1487), 1);
        assert_eq!(expected_chunks(441 * 1488), 2);
        // 30 s is 1501 frames: two chunks, as beat-this runs it.
        assert_eq!(expected_chunks(22_050 * 30), 2);
        assert_eq!(expected_chunks(22_050 * 240), 9);
    }
}
