//! What a grid is solved from, kept so it can be solved again, and the user's edits to it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::Meter;
use crate::units::{Bpm, SampleIndex};

/// Onsets at the analysis rate, as parallel arrays (compact in JSON).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct OnsetList {
    /// Rate `frames` count at: the analysis rate (22 050 Hz), not the file's.
    pub sample_rate: u32,
    /// Onset positions in frames at `sample_rate`.
    pub frames: Vec<u32>,
    /// Energy rise of each onset in dB (how sharp the attack is).
    pub rise_db: Vec<f32>,
    /// Envelope level just after each rise, in dBFS (how loud the attack is).
    pub level_db: Vec<f32>,
}

/// The cached inputs of the grid solver and the meter estimator, exactly as they consumed them,
/// so a refit (a user's edit, or none) reproduces the analysed grid without decoding again.
///
/// Positions here are not native-rate [`SampleIndex`] values: the model's times are kept as the
/// `f32` seconds it returned and onsets as frames at the analysis rate, because converting
/// either to the file's rate and back would not give the same numbers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GridEvidence {
    /// Model beat times in seconds, as the tracker returned them.
    pub beats_s: Vec<f32>,
    /// Model downbeat times in seconds, as the tracker returned them.
    pub downbeats_s: Vec<f32>,
    /// Downbeat activation (raw logits) per 20 ms frame (50 fps).
    pub downbeat_logits_50fps: Vec<f32>,
    /// Kick-band (30-150 Hz) onsets.
    pub kick_onsets: OnsetList,
    /// Broadband onsets, the anchor's fallback when the kick band is silent.
    pub broadband_onsets: OnsetList,
}

/// A user's corrections to an analysed grid, kept as overrides rather than as a finished grid
/// so they still apply when the file is analysed again. The default is no edit.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", default)]
pub struct GridEdit {
    /// The meter, replacing the estimated one.
    pub meter: Option<Meter>,
    /// A typed tempo at the meter's unit, kept exactly (never snapped).
    pub bpm: Option<Bpm>,
    /// A tapped tempo: the grid takes the lattice of the fitted tempo nearest to it.
    pub tempo_hint: Option<Bpm>,
    /// Octave steps from the chosen tempo: +1 doubles it, -1 halves it.
    pub octave: i8,
    /// A bar line placed by the user, at the native rate; any bar line may be given, bar 1 is
    /// the first one at or after the first beat.
    pub anchor: Option<SampleIndex>,
    /// Which beat of the bar the solver finds with the other overrides is beat 1 (0 = as
    /// solved), when no anchor is placed. It is relative: a later meter or tempo change moves
    /// the bar it counts from, so an editor that wants bar 1 to stay put places `anchor`.
    pub downbeat_shift: u8,
}

/// Tempos an edit may set or produce, in BPM at the meter's unit: wide enough for any pulse a
/// DJ counts, narrow enough that a grid never has more than about 17 lines per second.
pub const EDIT_BPM_LIMITS: (f64, f64) = (20.0, 1000.0);

/// The most octave steps an edit may take from the chosen tempo.
pub const EDIT_OCTAVE_LIMIT: i8 = 3;

/// The most pulses a bar of an edited meter may have.
pub const EDIT_MAX_PULSES: u8 = 32;

impl GridEdit {
    /// No override is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Checks the overrides against [`EDIT_BPM_LIMITS`], [`EDIT_OCTAVE_LIMIT`] and a meter of
    /// 1 to [`EDIT_MAX_PULSES`] pulses whose groups sum to the bar.
    ///
    /// # Errors
    /// [`crate::Error::InvalidArgument`] naming the first value out of its limits.
    pub fn validate(&self) -> crate::Result<()> {
        let invalid = |what: String| Err(crate::Error::InvalidArgument(what));
        let (lo, hi) = EDIT_BPM_LIMITS;
        for (name, bpm) in [("BPM", self.bpm), ("tapped BPM", self.tempo_hint)] {
            if let Some(Bpm(b)) = bpm
                && !(lo..=hi).contains(&b)
            {
                return invalid(format!("{name} {b} is outside {lo}-{hi}"));
            }
        }
        if self.octave.unsigned_abs() > EDIT_OCTAVE_LIMIT.unsigned_abs() {
            return invalid(format!(
                "{} octave steps (at most {EDIT_OCTAVE_LIMIT})",
                self.octave
            ));
        }
        if let Some(m) = &self.meter {
            let sum: u32 = m.grouping.iter().map(|&g| u32::from(g)).sum();
            if m.beats_per_bar == 0
                || m.beats_per_bar > EDIT_MAX_PULSES
                || m.grouping.contains(&0)
                || sum != u32::from(m.beats_per_bar)
            {
                return invalid(format!("meter {m:?}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact values are intended in these tests
    use super::*;
    use crate::analysis::{AnalysisRecord, BeatUnit};

    fn record_with(evidence: Option<GridEvidence>) -> AnalysisRecord {
        let json = r#"{"schema":2,"version":"0","path":"/a.wav","size":1,"mtimeNs":2,
            "spec":{"sampleRate":44100,"channels":2},"frames":1,"duration":1.0,"delay":0,
            "padding":0,"loudness":{"integrated":null,"momentaryMax":null,"shortTermMax":null,
            "shortTermP95":null,"shortTermTop30":null,"lra":null,"truePeak":-1.0,
            "samplePeak":-1.0,"plr":null,"dualMono":false,"timeline":{"hopMs":100,
            "shortTerm":[]}},"grid":null,"gridSkipped":null,"tags":{"bpm":null,"genre":null,
            "title":null,"artist":null},"evidence":null}"#;
        let mut record: AnalysisRecord = serde_json::from_str(json).unwrap();
        record.evidence = evidence;
        record
    }

    #[test]
    fn evidence_round_trips_exactly_through_json() {
        let onsets = OnsetList {
            sample_rate: 22_050,
            frames: vec![0, 11_025, u32::MAX],
            rise_db: vec![6.123_456_7, 30.0, f32::MIN_POSITIVE],
            level_db: vec![-80.0, -0.000_123_4, -12.345_678],
        };
        let evidence = GridEvidence {
            beats_s: vec![0.1, 0.52, 1.0 / 3.0, 123.456_79],
            downbeats_s: vec![0.52],
            downbeat_logits_50fps: vec![-3.256_789_4, 0.0, 2.123_456_7],
            kick_onsets: onsets.clone(),
            broadband_onsets: onsets,
        };
        let record = record_with(Some(evidence));
        let json = serde_json::to_string(&record).unwrap();
        let back: AnalysisRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.evidence, record.evidence, "every f32 bit survives");
    }

    #[test]
    fn grid_edits_are_checked_against_their_limits() {
        assert!(GridEdit::default().validate().is_ok());
        let ok = GridEdit {
            bpm: Some(Bpm(399.0)),
            octave: -3,
            meter: Some(Meter::ten_eight()),
            ..GridEdit::default()
        };
        assert!(ok.validate().is_ok());
        for bad in [
            GridEdit {
                bpm: Some(Bpm(f64::INFINITY)),
                ..GridEdit::default()
            },
            GridEdit {
                tempo_hint: Some(Bpm(1000.5)),
                ..GridEdit::default()
            },
            GridEdit {
                octave: 4,
                ..GridEdit::default()
            },
            GridEdit {
                meter: Some(Meter {
                    beats_per_bar: 0,
                    unit: BeatUnit::Quarter,
                    grouping: vec![],
                }),
                ..GridEdit::default()
            },
            GridEdit {
                meter: Some(Meter {
                    beats_per_bar: 4,
                    unit: BeatUnit::Quarter,
                    grouping: vec![1, 0, 3],
                }),
                ..GridEdit::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_grid_edit_defaults_to_no_edit_and_reads_missing_fields() {
        assert!(GridEdit::default().is_empty());
        let edit: GridEdit = serde_json::from_str(r#"{"octave":1}"#).unwrap();
        assert_eq!(edit.octave, 1);
        assert!(!edit.is_empty());
        assert_eq!(edit.anchor, None);
        let json = serde_json::to_string(&GridEdit {
            meter: Some(Meter::nine_eight_aksak()),
            anchor: Some(SampleIndex(44_100)),
            ..GridEdit::default()
        })
        .unwrap();
        assert!(json.contains("\"tempoHint\":null"), "{json}");
        assert!(json.contains("\"downbeatShift\":0"), "{json}");
    }
}
