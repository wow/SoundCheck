//! DECIDE: what processing would do to one analysed file under the batch's loudness settings.
//! Pure and cheap (well under a microsecond per row), so the whole table is replanned whenever
//! the settings change.
//!
//! Gain only. Turning down is always allowed. Turning up stops at the true-peak ceiling and the
//! rest is reported as "short by". MP3 moves in whole `global_gain` steps of 1.5051 dB (the
//! nearest step, capped by the ceiling on the way up), and the remainder is reported as
//! residual.

use sc_core::analysis::AnalysisRecord;
use sc_core::ipc::JobStage;
use sc_core::plan::{
    Codec, DecideSettings, GLOBAL_GAIN_STEP_DB, GainPlan, LoudnessMode, NEGLIGIBLE_DB, Plan,
    ReviewReason, SkipReason, TAG_BPM_TOLERANCE,
};
use sc_core::{Confidence, DbTp, Verdict};

/// What the grid solver writes when it looked for beats and found none.
const NO_BEATS: &str = "no beats found";

/// The plan for `record`, whose audio is `codec`, under `settings`. A grid the user confirmed by
/// ear (`grid_confirmed`) needs no review whatever its flags; the row still shows them.
#[must_use]
pub fn decide(
    record: &AnalysisRecord,
    codec: Codec,
    settings: &DecideSettings,
    grid_confirmed: bool,
) -> Plan {
    let loudness = &record.loudness;
    let measured = match settings.mode {
        LoudnessMode::Dj => loudness.short_term_p95,
        LoudnessMode::Streaming => loudness.integrated,
    };
    let skip = if !codec.has_gain_plan() {
        Some(SkipReason::AnalyseOnly { codec })
    } else if measured.is_none() {
        Some(SkipReason::Silent)
    } else {
        None
    };
    let gain = match (skip, measured) {
        (None, Some(m)) => Some(gain_plan(
            settings.target.0 - m.0,
            loudness.true_peak,
            settings.ceiling,
            codec == Codec::Mp3,
        )),
        _ => None,
    };
    let review = if grid_confirmed {
        Vec::new()
    } else {
        review_reasons(record, settings)
    };
    let status = if skip.is_some() {
        JobStage::Skipped
    } else if review.is_empty() {
        JobStage::Analysed
    } else {
        JobStage::NeedsReview
    };
    Plan {
        measured,
        gain,
        skip,
        review,
        status,
    }
}

/// The gain that moves a statistic by `desired` dB without a boost crossing `ceiling`.
fn gain_plan(desired: f64, true_peak: DbTp, ceiling: DbTp, mp3: bool) -> GainPlan {
    if desired.abs() < NEGLIGIBLE_DB {
        return GainPlan::AtTarget;
    }
    let headroom = (ceiling.0 - true_peak.0).max(0.0);
    if mp3 {
        // Step counts are tiny; the rounding is the intended quantisation.
        #[allow(clippy::cast_possible_truncation)]
        let mut steps = (desired / GLOBAL_GAIN_STEP_DB).round() as i32;
        if steps > 0 {
            #[allow(clippy::cast_possible_truncation)]
            let max_up = (headroom / GLOBAL_GAIN_STEP_DB).floor() as i32;
            steps = steps.min(max_up);
        }
        let gain_db = f64::from(steps) * GLOBAL_GAIN_STEP_DB;
        return GainPlan::GlobalGain {
            steps,
            gain_db,
            residual_lu: desired - gain_db,
            true_peak_after: DbTp(true_peak.0 + gain_db),
        };
    }
    let gain_db = if desired <= 0.0 {
        desired
    } else {
        desired.min(headroom)
    };
    let short_by = desired - gain_db;
    GainPlan::Gain {
        gain_db,
        short_by_lu: if short_by < NEGLIGIBLE_DB {
            0.0
        } else {
            short_by
        },
        true_peak_after: DbTp(true_peak.0 + gain_db),
    }
}

/// What a human should check: an unsure grid, a moving tempo, a BPM the DJ app will not show
/// as analysed, a contradicting tag, or no grid at all. A grid that fits with elevated residuals
/// (`StaticWarn`) is not on the list; the table marks it in the BPM column instead.
fn review_reasons(record: &AnalysisRecord, settings: &DecideSettings) -> Vec<ReviewReason> {
    let mut reasons = Vec::new();
    let Some(grid) = &record.grid else {
        if record.grid_skipped.as_deref() == Some(NO_BEATS) {
            reasons.push(ReviewReason::NoGrid);
        }
        return reasons;
    };
    if grid.confidence != Confidence::Green {
        reasons.push(ReviewReason::Confidence);
    }
    // Elevated residuals alone (StaticWarn) are shown in the BPM column, not queued for review:
    // the grid fits, and on a real library a third of the tracks land there.
    if grid.verdict == Verdict::Drifts {
        reasons.push(ReviewReason::Drifts);
    }
    let (lo, hi) = settings.bpm_range;
    if grid.bpm.0 < lo.0 || grid.bpm.0 > hi.0 {
        reasons.push(ReviewReason::OutsideBpmRange);
    }
    if let Some(tag) = record.tags.bpm
        && (grid.bpm.0 - tag.0).abs() > TAG_BPM_TOLERANCE * tag.0
    {
        reasons.push(ReviewReason::TagBpmDisagrees { tag });
    }
    reasons
}

#[cfg(test)]
mod tests;
