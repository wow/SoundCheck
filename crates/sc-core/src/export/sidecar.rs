//! What an export records about one written file in its sidecar (`<file>.soundcheck.json`,
//! key `export`): the settings and plans it was written with, the grid as exported, in the
//! output's samples, and the source's measurements the gain was planned from. A later run
//! (checking the exported grid, writing a rekordbox XML, undoing) reads it back instead of
//! planning again.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{ExportPlan, ExportSettings, GridCheck};
use crate::analysis::{AnalysisRecord, Meter};
use crate::plan::{DecideSettings, Plan};
use crate::units::{Bpm, DbTp, Lu, Lufs, SampleIndex, Seconds};

/// What an export did to one file and why, as its sidecar records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    /// The batch's export settings.
    pub settings: ExportSettings,
    /// The loudness settings the gain was decided with.
    pub decide: DecideSettings,
    /// The file's plan under `decide` (the statistic measured, the gain, any shortfall).
    pub decided: Plan,
    /// What the export wrote, planned from the cut the renderer made.
    pub plan: ExportPlan,
    /// The grid as exported, in the output's samples; `None` without a grid, or when it was
    /// withheld because it needed review ([`ExportPlan::grid_withheld`]).
    pub grid: Option<ExportedGrid>,
    /// The source's measurements.
    pub source: SourceMeasurements,
    /// The check of the exported grid against the written file's own analysis, recorded after
    /// the write; absent until it ran, and in records written before it existed.
    /// Readers of sidecars drop a value they cannot read (a later version's result), so it
    /// reads as absent (`sc_io`'s sidecar reader does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_check: Option<GridCheck>,
}

/// The grid of an exported file, in the output's samples (after the cut).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportedGrid {
    /// Bar 1 as shown (the grid's anchor) minus the frames cut.
    pub bar1: SampleIndex,
    /// The first bar line of the output, extrapolated from bar 1 by whole bars at the written
    /// tempo and rounded to a sample: after a Prepare cut, the lead (up to 1 ms and a sample
    /// later). The `SOUNDCHECK` record's `bar1`.
    pub first_bar_line: SampleIndex,
    /// The tempo at the meter's unit as written: two decimals ([`Bpm::written`]).
    pub bpm: Bpm,
    /// The tempo as fitted or typed, before rounding.
    pub bpm_exact: Bpm,
    /// The meter.
    pub meter: Meter,
    /// The grid is the user's edit of the analysed one.
    pub edited: bool,
    /// The user confirmed the grid by ear.
    pub confirmed: bool,
    /// Where the source's own analysis puts its bar line nearest `bar1` (in the output's
    /// samples), minus `bar1`, in milliseconds: the detector's grid solved with the user's
    /// meter, tempo octave and fitted part, but none of the bar line placed, the beat 1 chosen
    /// or the tempo typed. Zero for a grid the user did not edit. The check of the written file
    /// expects the same offset from its own analysis. Absent in records written before it and
    /// when the source's evidence was not at hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector_offset_ms: Option<f64>,
    /// That grid's tempo; absent as `detector_offset_ms` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector_bpm: Option<Bpm>,
}

/// The source's measurements an export planned from (the loudness timeline left out).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SourceMeasurements {
    /// Playable frames.
    #[ts(type = "number")]
    pub frames: u64,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u16,
    /// Playing time.
    pub duration: Seconds,
    /// Integrated loudness; `None` for silence.
    pub integrated: Option<Lufs>,
    /// S-P95, the DJ statistic.
    pub short_term_p95: Option<Lufs>,
    /// Power mean of the loudest 30 s of short-term windows.
    pub short_term_top30: Option<Lufs>,
    /// Loudest momentary window.
    pub momentary_max: Option<Lufs>,
    /// Loudest short-term window.
    pub short_term_max: Option<Lufs>,
    /// Loudness range.
    pub lra: Option<Lu>,
    /// True peak.
    pub true_peak: DbTp,
}

impl SourceMeasurements {
    /// The measurements of `record`.
    #[must_use]
    pub fn of(record: &AnalysisRecord) -> Self {
        let l = &record.loudness;
        Self {
            frames: record.frames,
            sample_rate: record.spec.sample_rate,
            channels: record.spec.channels,
            duration: record.duration,
            integrated: l.integrated,
            short_term_p95: l.short_term_p95,
            short_term_top30: l.short_term_top30,
            momentary_max: l.momentary_max,
            short_term_max: l.short_term_max,
            lra: l.lra,
            true_peak: l.true_peak,
        }
    }
}
