//! PLAN EXPORT: what exporting does to one analysed file, from its analysis, its decided gain
//! plan, the grid as shown (the user's edits applied) and the batch's export settings. Pure and
//! cheap, so the confirm summary and every row's Action can be computed before anything runs.
//!
//! - **Gain** is the decided plan's: gain only, a boost capped by the true-peak ceiling (the
//!   shortfall stays in the plan). Grid only applies none and keeps the source depth, so the
//!   samples stay bit for bit.
//! - **Prepare cut**: bar lines are extrapolated from bar 1 by whole bars (a bar is the meter's
//!   pulse count times `60 * sample_rate / bpm` samples). The earliest bar line `B` at or after
//!   the lead is the one the output starts on; `floor(B - lead)` frames are cut, so it lands at
//!   the lead or less than one sample after it (never before). When that would remove more than
//!   the bar's last beat (the pulses of its last group), nothing is cut: bar 1 lies further in,
//!   and removing more would remove music. Library mode never cuts.
//! - **Tags** (by neutral name, only into a tag the file has): `BPM` at the meter's unit with
//!   two decimals when the tempo tag is on; Replay Gain 2.0 track gain `-18 - (I + g)` dB and
//!   track peak `10^((TP + g) / 20)` when a gain is written; the `SOUNDCHECK` record always.
//! - **`bext`** (WAV): the measured loudness moved by the gain.

use sc_core::analysis::{AnalysisRecord, Grid};
use sc_core::export::{
    BatchMode, Cut, DEFAULT_LEAD_MS, ExportOutcome, ExportPlan, ExportSettings, ExportSkip,
    LEAD_MS_RANGE, Place, REPLAYGAIN_REFERENCE, SoundcheckRecord, TAG_BPM,
    TAG_REPLAYGAIN_TRACK_GAIN, TAG_REPLAYGAIN_TRACK_PEAK, TAG_SOUNDCHECK, XmlOnlyReason,
    positive_zero,
};
use sc_core::plan::{Codec, DecideSettings, GainPlan, Plan, SkipReason};
use sc_core::{BextLoudness, DbTp, Lu, Lufs, SampleIndex, Seconds, Tag, VERSION};
use sc_io::render::DJ_SAFE_RATES_HZ;

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
    /// The file holds Serato data (its cue points are positions in the audio).
    pub serato: bool,
    /// BLAKE3 of the source file, when known; the `SOUNDCHECK` record names it.
    pub blake3: Option<[u8; 32]>,
}

/// One file to plan.
#[derive(Debug, Clone, Copy)]
pub struct ExportInput<'a> {
    /// Its analysis, with the grid as shown (the user's edits applied).
    pub record: &'a AnalysisRecord,
    /// Its plan under `decide` ([`crate::decide()`]).
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
/// players refuse and silence are skipped; grid only needs a grid, and leaves FLAC, sources it
/// would requantise and files without a tag to the XML; a Prepare cut in place of a file with
/// Serato data is skipped; everything else is written.
#[must_use]
pub fn plan_export(input: &ExportInput<'_>, settings: &ExportSettings) -> ExportOutcome {
    let ExportInput { record, source, .. } = *input;
    let codec = source.codec;
    if matches!(codec, Codec::Mp3 | Codec::Aac) {
        return xml_only(XmlOnlyReason::Mp3OrAac { codec });
    }
    if !codec.is_writable() {
        return skip(ExportSkip::Unsupported { codec });
    }
    let sample_rate_hz = record.spec.sample_rate;
    if !DJ_SAFE_RATES_HZ.contains(&sample_rate_hz) {
        return skip(ExportSkip::NotDjSafeRate { sample_rate_hz });
    }
    if settings.grid_only {
        if let Some(reason) = grid_only_refusal(record, source) {
            return reason;
        }
    } else if input.plan.skip == Some(SkipReason::Silent) || input.plan.measured.is_none() {
        return skip(ExportSkip::Silent);
    }
    let (cut, trim_frames) = head(record, settings);
    if source.serato && settings.place == Place::InPlace && trim_frames > 0 {
        return skip(ExportSkip::SeratoInPlaceCut {
            cut_s: SampleIndex(trim_frames).to_seconds(sample_rate_hz),
        });
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
        .map(|bars| bar1_after(&bars, trim_frames, sample_rate_hz));
    let tags = tags(input, settings, gain_db, trim_frames, bar1);
    let bext = (!settings.grid_only && codec == Codec::Wav).then(|| bext(record, gain_db));
    ExportOutcome::Write {
        plan: ExportPlan {
            gain_db,
            trim_frames,
            expect_frames: record.frames.saturating_sub(trim_frames),
            bits: if settings.grid_only {
                None
            } else {
                settings.depth
            },
            tags,
            bext,
            cut,
        },
    }
}

fn xml_only(reason: XmlOnlyReason) -> ExportOutcome {
    ExportOutcome::XmlOnly { reason }
}

fn skip(reason: ExportSkip) -> ExportOutcome {
    ExportOutcome::Skip { reason }
}

/// Why a grid-only export cannot write this file, if it cannot.
fn grid_only_refusal(record: &AnalysisRecord, source: &ExportSource) -> Option<ExportOutcome> {
    if record.grid.is_none() {
        return Some(skip(ExportSkip::NoGrid));
    }
    if source.codec == Codec::Flac {
        return Some(xml_only(XmlOnlyReason::GridOnlyFlac));
    }
    // The writer keeps the source depth only for integer sources of up to 24 bits.
    if source.float || source.bits_per_sample.is_some_and(|b| b > 24) {
        return Some(xml_only(XmlOnlyReason::GridOnlyWouldRequantise {
            float: source.float,
            bits: source.bits_per_sample,
        }));
    }
    if !source.has_tag {
        return Some(xml_only(XmlOnlyReason::NoTagToWriteGridOnly));
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
    /// The bar lines of `grid` at `sample_rate`; `None` for a tempo that is not a positive
    /// number.
    pub(crate) fn of(grid: &Grid, sample_rate: u32) -> Option<Self> {
        let pulse = grid.samples_per_beat(sample_rate);
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

/// What happens at the start of the file, and the frames cut.
fn head(record: &AnalysisRecord, settings: &ExportSettings) -> (Cut, u64) {
    if settings.grid_only {
        return (Cut::GridOnly, 0);
    }
    if settings.batch_mode == BatchMode::Library {
        return (Cut::Library, 0);
    }
    let rate = record.spec.sample_rate;
    let Some(bars) = record.grid.as_ref().and_then(|g| Bars::of(g, rate)) else {
        return (Cut::NoGrid, 0);
    };
    let lead = lead_samples(settings.lead_ms, rate);
    let trim = floor_frames(bars.first_at_or_after(lead) - lead);
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let too_far = trim as f64 > bars.last_beat;
    if too_far || trim >= record.frames {
        let first = SampleIndex(floor_frames(bars.first_at_or_after(0.0).round()));
        return (
            Cut::NotCut {
                first_bar_line: first,
                first_bar_line_s: first.to_seconds(rate),
            },
            0,
        );
    }
    (
        Cut::Cut {
            frames: trim,
            seconds: SampleIndex(trim).to_seconds(rate),
        },
        trim,
    )
}

/// The first bar line of the exported audio (cut by `trim` frames), seconds.
fn bar1_after(bars: &Bars, trim: u64, sample_rate: u32) -> Seconds {
    // u64 -> f64 is exact below 2^53 samples.
    #[allow(clippy::cast_precision_loss)]
    let trim = trim as f64;
    Seconds((bars.first_at_or_after(trim) - trim) / f64::from(sample_rate))
}

/// The tag items, in a fixed order: `BPM`, Replay Gain track gain and peak, `SOUNDCHECK`.
fn tags(
    input: &ExportInput<'_>,
    settings: &ExportSettings,
    gain_db: f64,
    trim_frames: u64,
    bar1: Option<Seconds>,
) -> Vec<Tag> {
    let record = input.record;
    let bpm = record.grid.as_ref().map(|g| g.bpm);
    let mut tags = Vec::with_capacity(4);
    if settings.tbpm
        && let Some(bpm) = bpm
    {
        tags.push(Tag::new(TAG_BPM, format!("{:.2}", bpm.0)));
    }
    if !settings.grid_only
        && let Some(integrated) = record.loudness.integrated
    {
        let (gain, peak) = replaygain(integrated, record.loudness.true_peak, gain_db);
        tags.push(Tag::new(TAG_REPLAYGAIN_TRACK_GAIN, gain));
        tags.push(Tag::new(TAG_REPLAYGAIN_TRACK_PEAK, peak));
    }
    let soundcheck = SoundcheckRecord {
        app: VERSION,
        mode: settings.batch_mode,
        stat: input.decide.mode,
        target: input.decide.target,
        gain_db,
        trim_frames,
        bpm,
        bar1,
        source_blake3: input.source.blake3,
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
