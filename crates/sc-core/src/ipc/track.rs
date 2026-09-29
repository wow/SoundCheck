//! The grid view's IPC: opening a track, its event channel (analysis, decoding, playback), the
//! header of a refit's binary answer, and the row a saved edit gives. Waveform bins and residuals
//! travel as raw bytes (`tauri::ipc::Response`), never as JSON arrays of numbers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{FileEntry, IpcError, RowAnalysis};
use crate::analysis::{Grid, GridEdit, Timeline};
use crate::plan::Plan;
use crate::units::{Bpm, DbFs, SampleIndex};

/// What `track_open` answers: everything the grid view draws before the audio has decoded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TrackOpened {
    /// The row's file.
    pub entry: FileEntry,
    /// The file's sample rate, which every position counts in.
    pub sample_rate: u32,
    /// Playable frames.
    #[ts(type = "number")]
    pub frames: u64,
    /// The gain the player plays at: the row's planned gain, 0 dB for a skipped file.
    pub gain: DbFs,
    /// The DJ-app BPM range the grid is solved under.
    pub bpm_range: (Bpm, Bpm),
    /// The analysis's grid; `None` when no beats were found.
    pub analysed: Option<Grid>,
    /// The grid shown: the saved edit's, else the analysis's.
    pub grid: Option<Grid>,
    /// The saved edit (empty when none).
    pub edit: GridEdit,
    /// The grid was confirmed by ear.
    pub confirmed: bool,
    /// Short-term loudness over time, for the cursor readout.
    pub timeline: Timeline,
    /// The embedded cover's media type, when `track_cover` has one.
    pub cover: Option<String>,
}

/// What the grid view's channel carries, in order: `analysing` (only when the file had to be
/// analysed again), `decoded` at most every 100 ms, then `ready` or `failed`; `player` at 30 Hz
/// while playing and once when it stops; `playerError` when the output device fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TrackEvent {
    /// Analysis progress, `0.0..=1.0`.
    Analysing {
        /// Fraction done.
        fraction: f32,
    },
    /// Frames decoded so far.
    Decoded {
        /// Frames.
        #[ts(type = "number")]
        frames: u64,
    },
    /// The whole file is decoded.
    Ready {
        /// Frames.
        #[ts(type = "number")]
        frames: u64,
    },
    /// Decoding stopped; the frames decoded before stay viewable and playable.
    Failed {
        /// Why.
        error: IpcError,
        /// Frames decoded before it stopped.
        #[ts(type = "number")]
        frames: u64,
    },
    /// Where playback is.
    Player {
        /// Output is running.
        playing: bool,
        /// The frame being heard.
        position: SampleIndex,
        /// Device buffers that ran dry since the track was opened.
        #[ts(type = "number")]
        underruns: u64,
    },
    /// The output device failed; playback stopped.
    PlayerError {
        /// What to tell the user.
        message: String,
    },
}

/// The JSON header of `grid_refit`'s answer. The bytes are a little-endian `u32` header length,
/// the header, then one little-endian `f32` residual in milliseconds per grid line from
/// `first_line` (NaN where no attack is near).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GridFitHeader {
    /// The grid the edit gives; `None` when it gives none.
    pub grid: Option<Grid>,
    /// Line index of the first residual (bar 1's line is 0).
    #[ts(type = "number")]
    pub first_line: i64,
    /// Residuals that follow the header.
    pub lines: u32,
    /// The line where the smoothed residual curve is furthest from the grid.
    #[ts(type = "number | null")]
    pub worst_line: Option<i64>,
    /// Attacks matched to a line.
    pub matched: u32,
    /// Attacks within the judged span.
    pub attacks: u32,
    /// The two fits of a track whose tempo changes, compared on the start of the music; `None`
    /// unless the edit fits the start, the whole-track grid drifts, or the start fit holds at
    /// least 0.2 more of the start window's lines than the whole-track fit. Also `None` when the
    /// track has no start fit (too few attacks at its start); a start edit then gives the
    /// whole-track grid.
    pub fit_choice: Option<FitChoice>,
}

/// How well the whole-track fit and the start fit hold the start of the music: of the grid lines
/// in the start window, the share (0 to 1) that hold a kick-band attack (the broadband one when
/// the kick band is nearly silent) under each fit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FitChoice {
    /// Share of the start window's lines holding an attack under the whole-track fit.
    pub whole_share: f32,
    /// Share of the start window's lines holding an attack under the start fit.
    pub start_share: f32,
    /// End of the start window, in seconds from the start of the file (it begins at the first
    /// beat).
    pub window_end_s: f64,
}

/// What `grid_commit` answers: the row and its plan under the saved edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RowUpdate {
    /// The row.
    pub file_id: u32,
    /// The row's analysis with the edited grid.
    pub row: Box<RowAnalysis>,
    /// Its plan.
    pub plan: Plan,
    /// The settings revision the plan was decided with.
    pub revision: u32,
}
