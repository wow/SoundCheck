//! PLAN EXPORT: what exporting does to one analysed file, from its analysis, its decided gain
//! plan, the grid as shown (the user's edits applied) and the batch's export settings. Pure and
//! cheap, so the confirm summary and every row's Action can be computed before anything runs.
//!
//! - **Gain** is the decided plan's: gain only, a boost capped by the true-peak ceiling (the
//!   shortfall stays in the plan). Grid only applies none and keeps the source depth, so the
//!   samples stay bit for bit.
//! - **Prepare cut**: bar lines are extrapolated from bar 1 by whole bars (a bar is the meter's
//!   pulse count times `60 * sample_rate / bpm` samples, at the written BPM ([`Bpm::written`],
//!   two decimals): the tempo the tags and the XML carry, so a DJ app that lays its grid from
//!   the exported bar line at that tempo meets bar 1). Let `B` be the first bar line at or after the start. When
//!   `B` lies before the lead, the file already starts on a bar line and nothing is cut (no
//!   silence is ever added). Otherwise `floor(B - lead)` frames are asked to be cut, so `B`
//!   would land at the lead or less than one sample after it (never before); the renderer
//!   moves the cut up to 1 ms earlier to the quietest frame (never later) and fades the first
//!   2 ms in, so `B` lands between the lead and 1 ms (plus that sample) after it.
//!   [`plan_snapped_cut`] plans the cut, the frame count and the record from that snapped cut,
//!   which the render is then asked to make exactly. When the cut would remove more than the
//!   bar's last beat (the pulses of its last group), nothing is cut, because removing more
//!   would remove music. A grid that needs review and that the user has not confirmed is not
//!   cut to: where bar 1 is cannot be trusted. Library mode never cuts.
//! - **Tags** (by neutral name, only into a tag the file has): `BPM` at the meter's unit with
//!   two decimals when the tempo tag is on; Replay Gain 2.0 track gain `-18 - (I + g)` dB and
//!   track peak `10^((TP + g) / 20)` when a gain is written; the `SOUNDCHECK` record always.
//! - **`bext`** (WAV): the measured loudness moved by the gain.
//! - **XML off**: a file only the XML could carry is skipped with nothing to write.

use sc_core::analysis::{AnalysisRecord, Grid};
use sc_core::export::{
    BatchMode, Cut, DEFAULT_LEAD_MS, ExportNotice, ExportOutcome, ExportPlan, ExportSettings,
    ExportSkip, LEAD_MS_RANGE, Place, REPLAYGAIN_REFERENCE, RecordGain, SoundcheckRecord, TAG_BPM,
    TAG_REPLAYGAIN_TRACK_GAIN, TAG_REPLAYGAIN_TRACK_PEAK, TAG_SOUNDCHECK, XmlOnlyReason,
    positive_zero,
};
use sc_core::ipc::JobStage;
use sc_core::plan::{Codec, DecideSettings, GainPlan, Plan, SkipReason};
use sc_core::{BextLoudness, Bpm, DbTp, Lu, Lufs, SampleIndex, Tag, VERSION};
use sc_io::render::{DJ_SAFE_RATES_HZ, head_snap_frames};

/// What the planner needs to know about the source file beyond its analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportSource {
    /// The codec.
    pub codec: Codec,
    /// Significant bits per sample, when known.
    pub bits_per_sample: Option<u8>,
    /// Floating-point samples.
    pub float: bool,
    /// The file holds the one tag SoundCheck edits (WAV/AIFF: exactly one ID3 chunk); tags are
    /// never created. Only grid-only exports depend on it.
    pub has_tag: bool,
    /// Whether the file holds Serato data (its cue points are positions in the audio), as
    /// [`sc_io::probe`] detects it.
    pub serato: SeratoPresence,
    /// BLAKE3 of the source file, when known; the `SOUNDCHECK` record names it.
    pub blake3: Option<[u8; 32]>,
}

/// Whether a file holds Serato data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeratoPresence {
    /// None was found and every tag was read.
    #[default]
    Absent,
    /// Serato data was found.
    Present,
    /// The file or one of its tags could not be read, so Serato data cannot be ruled out: a cut
    /// in place is refused as if it held some.
    Unknown,
}

/// One file to plan.
#[derive(Debug, Clone, Copy)]
pub struct ExportInput<'a> {
    /// Its analysis, with the grid as shown (the user's edits applied).
    pub record: &'a AnalysisRecord,
    /// Its plan under `decide` ([`crate::decide()`]), decided with whether the user confirmed
    /// the grid: a confirmed grid needs no review, so only then is a grid that would otherwise
    /// need review cut to.
    pub plan: &'a Plan,
    /// The loudness settings the plan was decided with.
    pub decide: &'a DecideSettings,
    /// The source file.
    pub source: &'a ExportSource,
}

/// What exporting does with one file under `settings` (checked with
/// [`ExportSettings::validate`]; a lead outside its range is clamped into it).
///
/// In order: MP3/AAC are left to the XML; other codecs that are not written, sample rates DJ
/// players refuse, more than two channels and silence are skipped; grid only needs a grid, and
/// leaves FLAC, sources it would requantise and files without a tag to the XML; with the XML
/// off, what only the XML could carry is skipped; a Prepare cut in place of a file with Serato
/// data, or whose tags could not be read to rule it out, is skipped (a cut copy of a file with
/// Serato data gets a notice); everything else is written. That Serato skip is decided here
/// from the cut asked of the renderer; [`crate::plan_export_snapped`] decides it from the cut
/// made, so a cut that snaps back to the first frame (nothing cut) is not skipped.
#[must_use]
pub fn plan_export(input: &ExportInput<'_>, settings: &ExportSettings) -> ExportOutcome {
    serato_gate(input, settings, plan_ungated(input, settings))
}

/// [`plan_export`] without the Serato in-place skip.
pub(super) fn plan_ungated(input: &ExportInput<'_>, settings: &ExportSettings) -> ExportOutcome {
    let ExportInput { record, source, .. } = *input;
    let codec = source.codec;
    if matches!(codec, Codec::Mp3 | Codec::Aac) {
        return xml_only(settings, XmlOnlyReason::Mp3OrAac { codec });
    }
    if !codec.is_writable() {
        return skip(ExportSkip::Unsupported { codec });
    }
    let sample_rate_hz = record.spec.sample_rate;
    if !DJ_SAFE_RATES_HZ.contains(&sample_rate_hz) {
        return skip(ExportSkip::NotDjSafeRate { sample_rate_hz });
    }
    let channels = record.spec.channels;
    if channels > 2 {
        return skip(ExportSkip::UnsupportedChannels { channels });
    }
    if settings.grid_only {
        if let Some(reason) = grid_only_refusal(record) {
            return reason;
        }
        if let Some(reason) = grid_only_xml(source) {
            return xml_only(settings, reason);
        }
    } else if input.plan.skip == Some(SkipReason::Silent) || input.plan.measured.is_none() {
        return skip(ExportSkip::Silent);
    }
    let (cut, trim_frames) = head(record, input.plan, settings);
    ExportOutcome::Write {
        plan: write_plan(input, settings, cut, trim_frames, None),
    }
}

/// `outcome` (a plan of `input`), or the skip of a cut in place of a file with Serato data, or
/// whose tags could not be read to rule it out: its cue points are positions in the audio.
pub(super) fn serato_gate(
    input: &ExportInput<'_>,
    settings: &ExportSettings,
    outcome: ExportOutcome,
) -> ExportOutcome {
    let ExportOutcome::Write { plan } = &outcome else {
        return outcome;
    };
    if plan.trim_frames == 0 || settings.place != Place::InPlace {
        return outcome;
    }
    let cut_s = SampleIndex(plan.trim_frames).to_seconds(input.record.spec.sample_rate);
    match input.source.serato {
        SeratoPresence::Present => skip(ExportSkip::SeratoInPlaceCut { cut_s }),
        SeratoPresence::Unknown => skip(ExportSkip::SeratoUnknownInPlaceCut { cut_s }),
        SeratoPresence::Absent => outcome,
    }
}

/// The plan of `input` once its head cut is snapped: `plan` (what [`plan_export`] gave for
/// `input` and `settings`) with every value that depends on the cut planned from
/// `snapped_frames`, the cut the renderer makes for the planned one (what
/// [`sc_io::render::snap_head_cut`] gives for `plan.trim_frames`): `trim_frames`, the frame
/// count to expect, the cut shown, the `SOUNDCHECK` record's `trim=` and `bar1=`, the Serato
/// notice. Bar 1 then lands between the lead and 1 ms plus a sample after it. A snap to the
/// first frame cuts nothing and the file starts on its bar line. A plan without a cut is
/// returned as it is (its snap is 0).
///
/// Pure: the snap is the caller's to make, once, so that the render is asked for exactly this
/// cut ([`ExportPlan::trim_snapped_from_frames`] set) and never snaps it again.
///
/// # Errors
/// [`sc_core::Error::InvalidArgument`] when `plan` is already snapped, is not the plan of
/// `input` and `settings`, or `snapped_frames` is not a snap of its cut (later than it, or more
/// than [`head_snap_frames`] before it).
pub fn plan_snapped_cut(
    input: &ExportInput<'_>,
    settings: &ExportSettings,
    plan: &ExportPlan,
    snapped_frames: u64,
) -> sc_core::Result<ExportPlan> {
    let requested = plan.trim_frames;
    let invalid = |what: String| Err(sc_core::Error::InvalidArgument(what));
    if plan.trim_snapped_from_frames.is_some() {
        return invalid(format!(
            "the plan's cut of {requested} frames is already snapped"
        ));
    }
    let record = input.record;
    let (cut, trim) = head(record, input.plan, settings);
    if trim != requested || cut != plan.cut {
        return invalid(format!(
            "a plan cutting {requested} frames is not the plan of this file, which cuts {trim}"
        ));
    }
    if requested == 0 {
        if snapped_frames != 0 {
            return invalid(format!(
                "a plan without a cut snapped to {snapped_frames} frames"
            ));
        }
        return Ok(plan.clone());
    }
    let rate = record.spec.sample_rate;
    let window = head_snap_frames(rate);
    if snapped_frames > requested || snapped_frames < requested.saturating_sub(window) {
        return invalid(format!(
            "{snapped_frames} frames is not a snap of a {requested}-frame cut (at most \
             {window} frames earlier, never later)"
        ));
    }
    let cut = if snapped_frames == 0 {
        // `head` cut, so there is a grid with bar lines.
        match record
            .grid
            .as_ref()
            .and_then(|g| Bars::of(g, rate).map(|b| (g, b)))
        {
            Some((grid, bars)) => on_bar(&bars, grid.anchor, rate),
            None => return invalid("a cut planned without a grid".to_owned()),
        }
    } else {
        Cut::Cut {
            frames: snapped_frames,
            seconds: SampleIndex(snapped_frames).to_seconds(rate),
        }
    };
    Ok(write_plan(
        input,
        settings,
        cut,
        snapped_frames,
        Some(requested),
    ))
}

/// The plan of a file that is written with the head cut `cut` of `trim_frames` frames.
fn write_plan(
    input: &ExportInput<'_>,
    settings: &ExportSettings,
    cut: Cut,
    trim_frames: u64,
    trim_snapped_from_frames: Option<u64>,
) -> ExportPlan {
    let ExportInput { record, source, .. } = *input;
    let sample_rate_hz = record.spec.sample_rate;
    let mut notices = Vec::new();
    if trim_frames > 0 && source.serato == SeratoPresence::Present {
        // A cut in place of such a file was skipped; this is a copy in a folder.
        let cut_s = SampleIndex(trim_frames).to_seconds(sample_rate_hz);
        notices.push(ExportNotice::SeratoCuesShifted { cut_s });
    }
    let gain_db = if settings.grid_only {
        0.0
    } else {
        planned_gain(input.plan)
    };
    let bar1 = record
        .grid
        .as_ref()
        .and_then(|g| Bars::of(g, sample_rate_hz))
        .map(|bars| bar1_after(&bars, trim_frames));
    let tags = tags(input, settings, gain_db, trim_frames, bar1);
    let bext = (!settings.grid_only && source.codec == Codec::Wav).then(|| bext(record, gain_db));
    ExportPlan {
        gain_db,
        trim_frames,
        trim_snapped_from_frames,
        expect_frames: record.frames.saturating_sub(trim_frames),
        bits: if settings.grid_only {
            None
        } else {
            settings.depth
        },
        tags,
        bext,
        cut,
        notices,
    }
}

/// Left to the XML, or, with the XML off, skipped with nothing to write.
fn xml_only(settings: &ExportSettings, reason: XmlOnlyReason) -> ExportOutcome {
    if settings.xml {
        ExportOutcome::XmlOnly { reason }
    } else {
        skip(ExportSkip::NothingToWrite { reason })
    }
}

fn skip(reason: ExportSkip) -> ExportOutcome {
    ExportOutcome::Skip { reason }
}

/// Why a grid-only export skips this file, if it does.
fn grid_only_refusal(record: &AnalysisRecord) -> Option<ExportOutcome> {
    record.grid.is_none().then(|| skip(ExportSkip::NoGrid))
}

/// Why a grid-only export leaves this file to the XML, if it does.
fn grid_only_xml(source: &ExportSource) -> Option<XmlOnlyReason> {
    if source.codec == Codec::Flac {
        return Some(XmlOnlyReason::GridOnlyFlac);
    }
    // Only 16- and 24-bit integer samples are written back at their own depth; anything else
    // (float, 8-bit, 20 bits in 24, 32-bit, unknown) would change the PCM.
    if source.float || !matches!(source.bits_per_sample, Some(16 | 24)) {
        return Some(XmlOnlyReason::GridOnlyWouldRequantise {
            float: source.float,
            bits: source.bits_per_sample,
        });
    }
    if !source.has_tag {
        return Some(XmlOnlyReason::NoTagToWriteGridOnly);
    }
    None
}

/// The decided gain, dB; 0 at target or without a gain plan.
fn planned_gain(plan: &Plan) -> f64 {
    match plan.gain {
        Some(GainPlan::Gain { gain_db, .. } | GainPlan::GlobalGain { gain_db, .. }) => gain_db,
        Some(GainPlan::AtTarget) | None => 0.0,
    }
}

/// The bar lines of a static grid at the file's rate, in samples (fractional: a bar is rarely a
/// whole number of samples, and rounding it would drift by a sample per bar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bars {
    /// Bar 1, samples.
    pub anchor: f64,
    /// Length of one bar, samples.
    pub bar: f64,
    /// Length of the bar's last beat (its last group of pulses), samples.
    pub last_beat: f64,
}

impl Bars {
    /// The bar lines of `grid` at `sample_rate`, at its BPM rounded to two decimals (the tempo
    /// written); `None` for a tempo that does not round to a positive number.
    pub(crate) fn of(grid: &Grid, sample_rate: u32) -> Option<Self> {
        let pulse = 60.0 * f64::from(sample_rate) / grid.bpm.written().0;
        let pulses: u32 = grid.meter.grouping.iter().map(|&g| u32::from(g)).sum();
        let last = grid.meter.grouping.last().copied().unwrap_or(1);
        let bar = pulse * f64::from(pulses.max(1));
        // u64 -> f64 is exact below 2^53 samples (about 6,000 years at 48 kHz).
        #[allow(clippy::cast_precision_loss)]
        let anchor = grid.anchor.0 as f64;
        (pulse.is_finite() && pulse > 0.0).then_some(Self {
            anchor,
            bar,
            last_beat: pulse * f64::from(last),
        })
    }

    /// The earliest bar line at or after `pos` samples.
    pub(crate) fn first_at_or_after(&self, pos: f64) -> f64 {
        let bars_back = ((self.anchor - pos) / self.bar).floor();
        self.anchor - bars_back * self.bar
    }
}

/// `x` samples as whole frames, rounded down; negative and NaN give 0.
// `as` saturates: NaN and negative values give 0, values past u64 the maximum.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn floor_frames(x: f64) -> u64 {
    x.floor() as u64
}

/// The lead, samples at `sample_rate`.
fn lead_samples(lead_ms: f64, sample_rate: u32) -> f64 {
    let ms = if lead_ms.is_finite() {
        lead_ms.clamp(LEAD_MS_RANGE.0, LEAD_MS_RANGE.1)
    } else {
        DEFAULT_LEAD_MS
    };
    ms * f64::from(sample_rate) / 1000.0
}

/// What happens at the start of the file, and the frames cut. A grid that needs review (and
/// that the user has not confirmed, which `plan` already says: a confirmed grid needs none) is
/// not cut to.
fn head(record: &AnalysisRecord, plan: &Plan, settings: &ExportSettings) -> (Cut, u64) {
    if settings.grid_only {
        return (Cut::GridOnly, 0);
    }
    if settings.batch_mode == BatchMode::Library {
        return (Cut::Library, 0);
    }
    let rate = record.spec.sample_rate;
    let Some((grid, bars)) = record
        .grid
        .as_ref()
        .and_then(|g| Bars::of(g, rate).map(|b| (g, b)))
    else {
        return (Cut::NoGrid, 0);
    };
    let anchor = grid.anchor;
    if !grid_trusted(plan) {
        let cut = Cut::NeedsReview {
            bar1: anchor,
            bar1_s: anchor.to_seconds(rate),
        };
        return (cut, 0);
    }
    let lead = lead_samples(settings.lead_ms, rate);
    let first = bars.first_at_or_after(0.0);
    let trim = floor_frames(first - lead);
    if first < lead || trim == 0 {
        return (on_bar(&bars, anchor, rate), 0);
    }
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let too_far = trim as f64 > bars.last_beat;
    if too_far || trim >= record.frames {
        let first_line = first_line(&bars);
        let cut = Cut::NotCut {
            bar1: anchor,
            bar1_s: anchor.to_seconds(rate),
            first_bar_line: first_line,
            first_bar_line_s: first_line.to_seconds(rate),
        };
        return (cut, 0);
    }
    let cut = Cut::Cut {
        frames: trim,
        seconds: SampleIndex(trim).to_seconds(rate),
    };
    (cut, trim)
}

/// Whether the grid may be cut to and written: the row does not need review (a grid the user
/// confirmed never does; see [`crate::decide()`]).
fn grid_trusted(plan: &Plan) -> bool {
    plan.status != JobStage::NeedsReview
}

/// The first bar line at or after the start, rounded to a sample.
fn first_line(bars: &Bars) -> SampleIndex {
    SampleIndex(floor_frames(bars.first_at_or_after(0.0).round()))
}

/// The file starts on its first bar line: nothing is cut.
fn on_bar(bars: &Bars, anchor: SampleIndex, rate: u32) -> Cut {
    let line = first_line(bars);
    Cut::OnBar {
        bar_line: line,
        bar_line_s: line.to_seconds(rate),
        bar1: anchor,
        bar1_s: anchor.to_seconds(rate),
    }
}

/// The first bar line of the exported audio (cut by `trim` frames), rounded to a sample.
fn bar1_after(bars: &Bars, trim: u64) -> SampleIndex {
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let trim = trim as f64;
    SampleIndex(floor_frames((bars.first_at_or_after(trim) - trim).round()))
}

/// The tag items, in a fixed order: `BPM`, Replay Gain track gain and peak, `SOUNDCHECK`. A
/// grid that needs review (and was not confirmed) is not written: no tempo tag, and the record
/// says `bpm=none;bar1=none`. Writing its tempo would replace a tag that disagrees with it, and
/// the next analysis would then find nothing to review.
fn tags(
    input: &ExportInput<'_>,
    settings: &ExportSettings,
    gain_db: f64,
    trim_frames: u64,
    bar1: Option<SampleIndex>,
) -> Vec<Tag> {
    let record = input.record;
    let grid_withheld = record.grid.is_some() && !grid_trusted(input.plan);
    let bpm = record
        .grid
        .as_ref()
        .filter(|_| !grid_withheld)
        .map(|g| g.bpm);
    let bar1 = bar1.filter(|_| !grid_withheld);
    let mut tags = Vec::with_capacity(4);
    if settings.tbpm
        && let Some(bpm) = bpm
    {
        tags.push(Tag::new(TAG_BPM, format!("{:.2}", bpm.written().0)));
    }
    if !settings.grid_only
        && let Some(integrated) = record.loudness.integrated
    {
        let (gain, peak) = replaygain(integrated, record.loudness.true_peak, gain_db);
        tags.push(Tag::new(TAG_REPLAYGAIN_TRACK_GAIN, gain));
        tags.push(Tag::new(TAG_REPLAYGAIN_TRACK_PEAK, peak));
    }
    let soundcheck = SoundcheckRecord {
        app: VERSION.to_owned(),
        mode: settings.batch_mode,
        gain: (!settings.grid_only).then_some(RecordGain {
            stat: input.decide.mode,
            target: input.decide.target,
            gain_db,
        }),
        trim_frames,
        sample_rate: record.spec.sample_rate,
        bpm: bpm.map(Bpm::written),
        bar1,
        grid_withheld,
        source_hash: input
            .source
            .blake3
            .as_ref()
            .map(SoundcheckRecord::hash_prefix),
    };
    tags.push(Tag::new(TAG_SOUNDCHECK, soundcheck.to_value()));
    tags
}

/// Replay Gain 2.0 track gain (`"-7.00 dB"`: the change that brings the exported file to -18
/// LUFS) and track peak (linear, six decimals; the true peak after the gain).
#[must_use]
pub fn replaygain(integrated: Lufs, true_peak: DbTp, gain_db: f64) -> (String, String) {
    let track_gain = REPLAYGAIN_REFERENCE.0 - (integrated.0 + gain_db);
    let peak = 10f64.powf((true_peak.0 + gain_db) / 20.0);
    (
        format!("{:+.2} dB", positive_zero(track_gain)),
        format!("{peak:.6}"),
    )
}

/// The `bext` loudness of the exported file: the measurements moved by `gain_db` (the range is
/// a difference and does not move).
fn bext(record: &AnalysisRecord, gain_db: f64) -> BextLoudness {
    let l = &record.loudness;
    let moved = |v: Option<Lufs>| v.map(|x| Lufs(x.0 + gain_db));
    BextLoudness::from_measurements(
        moved(l.integrated),
        l.lra.map(|r| Lu(r.0)),
        Some(DbTp(l.true_peak.0 + gain_db)),
        moved(l.momentary_max),
        moved(l.short_term_max),
    )
}

#[cfg(test)]
mod tests;
