//! Saved grid edits applied where an analysis becomes a row: in the app's session and in
//! `sc-cli plan` and `labels`, so both show the same grids and count the same rows.
//!
//! An edit applies to the audio it was made on (same decoded length, sample rate and integrated
//! loudness, so tag writes by DJ apps keep it). Its grid is the file's evidence solved with the
//! edit's overrides under the BPM range it was made with, which is the grid the user saw even
//! after the range setting changes. A confirmation counts only while that grid is still the one
//! the user confirmed: a new beat model or solver that moves it sends the row back to review.

use sc_analysis::grid::{self, Solved};
use sc_analysis::refit::{self, Context};
use sc_core::analysis::{AnalysisRecord, Grid, GridEdit, GridFit, Verdict};
use sc_core::ipc::FitChoice;
use sc_core::{Bpm, Result, SampleIndex};
use sc_io::edits::{AudioIdentity, EDIT_SCHEMA, EditStore, GridPin, SavedEdit};

/// Largest tempo difference, in BPM, at which a refitted grid is still the one confirmed.
const PIN_BPM: f64 = 0.005;
/// Largest bar-1 difference, in samples, at which a refitted grid is still the one confirmed.
const PIN_SAMPLES: u64 = 1;

/// What a saved edit did to a record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditState {
    /// The grid differs from the analysis: the user's overrides were applied.
    pub edited: bool,
    /// The user confirmed this grid by ear.
    pub confirmed: bool,
    /// The part of the track the applied edit fits the grid to (whole when no edit applied).
    pub fit: GridFit,
}

/// The audio `record` describes.
#[must_use]
pub fn audio_of(record: &AnalysisRecord) -> AudioIdentity {
    AudioIdentity {
        frames: record.frames,
        sample_rate: record.spec.sample_rate,
        integrated: record.loudness.integrated,
    }
}

fn pin_of(grid: &Grid) -> GridPin {
    GridPin {
        bpm: grid.bpm,
        anchor: grid.anchor,
        meter: grid.meter.clone(),
    }
}

fn same_grid(pin: Option<&GridPin>, grid: Option<&Grid>) -> bool {
    match (pin, grid) {
        (Some(p), Some(g)) => {
            (p.bpm.0 - g.bpm.0).abs() <= PIN_BPM
                && p.anchor.0.abs_diff(g.anchor.0) <= PIN_SAMPLES
                && p.meter == g.meter
        }
        (None, None) => true,
        _ => false,
    }
}

/// The grid `edit` gives on `record` (evidence included) under `bpm_range`, with its per-line
/// residuals; `None` without evidence or when the edit gives no grid.
#[must_use]
pub fn refit_record(
    record: &AnalysisRecord,
    bpm_range: (Bpm, Bpm),
    edit: &GridEdit,
) -> Option<Solved> {
    let evidence = record.evidence.as_ref()?;
    let ctx = Context {
        bpm_range,
        tags: &record.tags,
        sample_rate: record.spec.sample_rate,
    };
    refit::refit(evidence, &ctx, edit)
}

/// The attacks a dragged bar 1 snaps to, in seconds: the kick-band onsets, or the broadband ones
/// when the kick band is nearly silent (the solver's own choice). Empty without evidence.
#[must_use]
pub fn snap_onsets(record: &AnalysisRecord) -> Vec<f64> {
    let Some(ev) = &record.evidence else {
        return Vec::new();
    };
    refit::grid_attacks(ev).iter().map(|o| o.time_s).collect()
}

/// A start fit is offered for a whole-track grid that does not drift when it holds at least
/// this much more of the start window's lines: the verdict counts only the attacks that match a
/// line, so a track whose start the whole-track grid misses can still read static.
const OFFER_SHARE_GAIN: f32 = 0.2;

/// The whole-track and the start fit of `record` under `edit` (made with `bpm_range`), compared
/// on the start of the music: the share of each grid's lines in the start window
/// ([`refit::start_window_s`]) that hold an attack (the attacks bar 1 snaps to). `Some` when
/// the edit fits the start, when its whole-track grid drifts, or when the start fit holds
/// [`OFFER_SHARE_GAIN`] more of the window than the whole-track fit. `None` without evidence,
/// without a start fit (no run of attacks, or too few, at the start: a start edit then gives the
/// whole-track grid, so there is nothing to compare) or when either fit gives no grid.
/// `solved` is the edit's own grid when the caller has it (it is refitted otherwise); the other
/// fit is refitted with the rest of the edit unchanged.
#[must_use]
pub fn fit_choice(
    record: &AnalysisRecord,
    bpm_range: (Bpm, Bpm),
    edit: &GridEdit,
    solved: Option<&Solved>,
) -> Option<FitChoice> {
    let evidence = record.evidence.as_ref()?;
    let window = refit::start_window_s(evidence)?;
    let own_refit;
    let current = if let Some(s) = solved {
        s
    } else {
        own_refit = refit_record(record, bpm_range, edit)?;
        &own_refit
    };
    let other_fit = match edit.fit {
        GridFit::Whole => GridFit::Start,
        GridFit::Start => GridFit::Whole,
    };
    let other = refit_record(
        record,
        bpm_range,
        &GridEdit {
            fit: other_fit,
            ..edit.clone()
        },
    )?;
    let (whole, start) = match edit.fit {
        GridFit::Whole => (current, &other),
        GridFit::Start => (&other, current),
    };
    let attacks = refit::grid_attacks(evidence);
    let rate = record.spec.sample_rate;
    let share = |s: &Solved| {
        to_f32(grid::line_share(
            &s.grid,
            &attacks,
            rate,
            window.from_s,
            window.to_s,
        ))
    };
    let choice = FitChoice {
        whole_share: share(whole),
        start_share: share(start),
        window_end_s: window.to_s,
    };
    let offered = edit.fit == GridFit::Start
        || whole.grid.verdict == Verdict::Drifts
        || choice.start_share >= choice.whole_share + OFFER_SHARE_GAIN;
    offered.then_some(choice)
}

/// A share (0 to 1) as `f32`.
#[allow(clippy::cast_possible_truncation)] // a share in 0..=1 fits f32
fn to_f32(share: f64) -> f32 {
    share as f32
}

/// An edit that gives no grid for a file that has one (or a file without evidence).
struct Unsolved;

/// The grid of `record` under `edit` made with `bpm_range`; `Ok(None)` for a file without a grid
/// and no edit.
fn solve(
    record: &AnalysisRecord,
    bpm_range: (Bpm, Bpm),
    edit: &GridEdit,
) -> std::result::Result<Option<Grid>, Unsolved> {
    let Some(evidence) = &record.evidence else {
        // Without evidence only "the analysis as it is" can be reproduced.
        return if edit.is_empty() {
            Ok(record.grid.clone())
        } else {
            Err(Unsolved)
        };
    };
    let ctx = Context {
        bpm_range,
        tags: &record.tags,
        sample_rate: record.spec.sample_rate,
    };
    match refit::refit(evidence, &ctx, edit) {
        Some(solved) => Ok(Some(solved.grid)),
        None if edit.is_empty() && record.grid.is_none() => Ok(None),
        None => Err(Unsolved),
    }
}

/// Replaces `record`'s grid with the grid of the edit saved for its file, when the edit was made
/// on the same audio and still gives a grid. A record without evidence (from an older cache
/// entry) takes only an edit that leaves the analysis as it is.
pub fn apply_saved(record: &mut AnalysisRecord, store: &EditStore) -> EditState {
    let Some(saved) = store.get(&record.path) else {
        return EditState::default();
    };
    if saved.audio != audio_of(record) {
        tracing::info!(file = %record.path, "grid edit made on other audio; not applied");
        return EditState::default();
    }
    let Ok(grid) = solve(record, saved.bpm_range, &saved.edit) else {
        return EditState::default();
    };
    let state = EditState {
        edited: grid != record.grid,
        confirmed: saved.confirmed && same_grid(saved.grid.as_ref(), grid.as_ref()),
        fit: saved.edit.fit,
    };
    if grid.is_some() {
        record.grid_skipped = None;
        record.grid = grid;
    }
    state
}

/// Saves `edit` of `record` (analysed with `bpm_range`, evidence included) with the grid it
/// gives, confirmed or not, and returns the record's state under it. An empty unconfirmed edit
/// removes the saved one.
///
/// # Errors
/// [`sc_core::Error::InvalidArgument`] when the edit is outside its limits or gives no grid on a
/// file that has one; [`sc_core::Error::Io`] when it cannot be written.
pub fn save_edit(
    store: &EditStore,
    record: &AnalysisRecord,
    bpm_range: (Bpm, Bpm),
    edit: &GridEdit,
    confirmed: bool,
) -> Result<EditState> {
    edit.validate()?;
    if edit.is_empty() && !confirmed {
        store.remove(&record.path)?;
        return Ok(EditState::default());
    }
    let grid = solve(record, bpm_range, edit).map_err(|Unsolved| {
        sc_core::Error::InvalidArgument("the edit gives no grid for this file".into())
    })?;
    store.put(&SavedEdit {
        schema: EDIT_SCHEMA,
        path: record.path.clone(),
        audio: audio_of(record),
        bpm_range,
        edit: edit.clone(),
        grid: grid.as_ref().map(pin_of),
        confirmed,
    })?;
    Ok(EditState {
        edited: grid != record.grid,
        confirmed,
        fit: edit.fit,
    })
}

/// Carries `saved`, the user's edit of a file that was then exported with its first `trim`
/// frames cut, over to the exported file: the edit saved for `output` (its NFC path; the same
/// path in place), made on `audio` (the output's own audio), with bar 1 moved back by the cut.
/// `grid` is the grid the edit gave on the source, as shown.
///
/// A grid the user confirmed is carried pinned: its tempo (typed, exact), meter and bar 1
/// become the edit, so the output's own analysis gives the very grid that was confirmed even
/// where it would solve the start of the cut file differently. An edit that was not confirmed
/// keeps its overrides (a placed bar line moves to bar 1, back by the cut), so the output's grid is solved, and
/// judged for review, as the source's was.
///
/// # Errors
/// [`sc_core::Error::InvalidArgument`] when bar 1 lies inside the cut; [`sc_core::Error::Io`]
/// when the edit cannot be written.
pub fn carry_edit(
    store: &EditStore,
    saved: &SavedEdit,
    grid: Option<&Grid>,
    confirmed: bool,
    trim: u64,
    output: &str,
    audio: AudioIdentity,
) -> Result<()> {
    let back = |anchor: SampleIndex| {
        anchor.0.checked_sub(trim).map(SampleIndex).ok_or_else(|| {
            sc_core::Error::InvalidArgument(format!(
                "bar 1 at sample {} lies inside the {trim}-frame cut",
                anchor.0
            ))
        })
    };
    let pin = grid
        .map(|g| {
            Ok::<_, sc_core::Error>(GridPin {
                anchor: back(g.anchor)?,
                ..pin_of(g)
            })
        })
        .transpose()?;
    let edit = match (&pin, confirmed) {
        (Some(p), true) => GridEdit {
            meter: Some(p.meter.clone()),
            bpm: Some(p.bpm),
            tempo_hint: None,
            octave: 0,
            anchor: Some(p.anchor),
            downbeat_shift: 0,
            fit: saved.edit.fit,
        },
        // A placed bar line (any line of bar 1's lattice, maybe one inside the cut) is placed
        // again at bar 1, which lies on the same lattice.
        _ => GridEdit {
            anchor: match (saved.edit.anchor, &pin) {
                (None, _) => None,
                (Some(_), Some(p)) => Some(p.anchor),
                (Some(a), None) => Some(back(a)?),
            },
            ..saved.edit.clone()
        },
    };
    store.put(&SavedEdit {
        schema: EDIT_SCHEMA,
        path: output.to_owned(),
        audio,
        bpm_range: saved.bpm_range,
        edit,
        grid: pin,
        confirmed,
    })
}
