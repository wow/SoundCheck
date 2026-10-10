//! Unit tests of `crates/sc-engine/src/export/plan.rs`; the cut rule is in `tests/cut.rs`.
//! Sample counts are exact: at 44.1 kHz, 120 BPM in 4/4 has a 22,050-sample beat and an
//! 88,200-sample bar, and a 5 ms lead is 220.5 samples; at 48 kHz they are 24,000, 96,000 and
//! 240.
use super::*;
use sc_core::analysis::{Alternatives, BeatUnit, LoudnessReport, Meter, TagHints, Timeline};
use sc_core::export::{BatchMode, ExportSettings, Place};
use sc_core::{AudioSpec, Bpm, Confidence, DbFs, Seconds, Verdict};

use crate::decide;

mod cut;

/// Four minutes at `rate` with S-P95 -9 and I -9 LUFS (a 2 dB cut at the DJ target of -11)
/// and a true peak of -1 dBTP; a 4/4 grid at 120 BPM with bar 1 at `anchor`.
fn record(rate: u32, anchor: u64) -> AnalysisRecord {
    AnalysisRecord {
        schema: sc_core::analysis::RECORD_SCHEMA,
        version: "test".into(),
        path: "/x.wav".into(),
        size: 1,
        mtime_ns: 1,
        spec: AudioSpec::new(rate, 2),
        frames: u64::from(rate) * 240,
        duration: Seconds(240.0),
        delay: 0,
        padding: 0,
        loudness: LoudnessReport {
            integrated: Some(Lufs(-9.0)),
            momentary_max: Some(Lufs(-6.0)),
            short_term_max: Some(Lufs(-7.0)),
            short_term_p95: Some(Lufs(-9.0)),
            short_term_top30: None,
            lra: Some(Lu(4.0)),
            true_peak: DbTp(-1.0),
            sample_peak: DbFs(-1.2),
            plr: Some(Lu(8.0)),
            dual_mono: false,
            timeline: Timeline::default(),
        },
        grid: Some(Grid {
            anchor: SampleIndex(anchor),
            bpm: Bpm(120.0),
            meter: Meter::four_four(),
            meter_runner_up: None,
            first_downbeat_index: 0,
            phrase_len_bars: 8,
            segments: Vec::new(),
            residual_p95_ms: 2.0,
            residual_max_ms: 4.0,
            local_bpm_range: 0.0,
            drift_ppm: 0.0,
            verdict: Verdict::Static,
            confidence: Confidence::Green,
            reasons: Vec::new(),
            alternatives: Alternatives::default(),
        }),
        grid_skipped: None,
        tags: TagHints::default(),
        evidence: None,
    }
}

/// A 24-bit WAV with a tag.
fn wav() -> ExportSource {
    ExportSource {
        codec: Codec::Wav,
        bits_per_sample: Some(24),
        has_tag: true,
        ..ExportSource::default()
    }
}

fn plan_with(record: &AnalysisRecord, source: &ExportSource, s: &ExportSettings) -> ExportOutcome {
    let decide_settings = DecideSettings::dj();
    let plan = decide(record, source.codec, &decide_settings, false);
    plan_export(
        &ExportInput {
            record,
            plan: &plan,
            decide: &decide_settings,
            source,
        },
        s,
    )
}

fn written(outcome: ExportOutcome) -> ExportPlan {
    match outcome {
        ExportOutcome::Write { plan } => plan,
        other => panic!("not written: {other:?}"),
    }
}

fn prepare() -> ExportSettings {
    ExportSettings::new(BatchMode::Prepare)
}

fn cut_of(rate: u32, anchor: u64) -> Cut {
    written(plan_with(&record(rate, anchor), &wav(), &prepare())).cut
}

fn tag<'a>(plan: &'a ExportPlan, name: &str) -> Option<&'a str> {
    plan.tags
        .iter()
        .find(|t| t.name == name)
        .map(|t| t.value.as_str())
}

#[test]
fn library_never_trims_and_expects_same_frames() {
    let r = record(44_100, 101_430);
    let plan = written(plan_with(
        &r,
        &wav(),
        &ExportSettings::new(BatchMode::Library),
    ));
    assert_eq!(plan.cut, Cut::Library);
    assert_eq!((plan.trim_frames, plan.expect_frames), (0, r.frames));
    assert!((plan.gain_db + 2.0).abs() < 1e-12);
    // The first bar line of the untouched file: 13,230 samples.
    let record = tag(&plan, "SOUNDCHECK").expect("record");
    assert!(
        record.contains(";mode=library;") && record.contains(";bar1=13230"),
        "{record}"
    );
}

#[test]
fn grid_only_zero_gain_exact() {
    let settings = ExportSettings {
        grid_only: true,
        depth: Some(16),
        ..prepare()
    };
    let plan = written(plan_with(&record(44_100, 101_430), &wav(), &settings));
    assert_eq!(plan.gain_db.to_bits(), 0.0_f64.to_bits());
    assert_eq!((plan.trim_frames, plan.bits, plan.bext), (0, None, None));
    assert_eq!(plan.cut, Cut::GridOnly);
    assert_eq!(tag(&plan, "REPLAYGAIN_TRACK_GAIN"), None);
    assert_eq!(tag(&plan, "REPLAYGAIN_TRACK_PEAK"), None);
    assert_eq!(tag(&plan, "BPM"), Some("120.00"));
    let value = tag(&plan, "SOUNDCHECK").expect("record");
    // No level was aligned: no statistic, no target, and never "+0.00".
    assert!(value.contains(";mode=prepare;gain=none;trim=0;"), "{value}");
    assert!(
        !value.contains("stat=") && !value.contains("target="),
        "{value}"
    );
    let at_target = ExportSettings {
        grid_only: false,
        ..settings
    };
    let mut r = record(44_100, 101_430);
    r.loudness.short_term_p95 = Some(Lufs(-11.0));
    let plan = written(plan_with(&r, &wav(), &at_target));
    assert!(
        tag(&plan, "SOUNDCHECK")
            .expect("record")
            .contains(";gain=+0.00;")
    );
}

#[test]
fn grid_only_flac_is_xml_only() {
    let flac = ExportSource {
        codec: Codec::Flac,
        ..wav()
    };
    let settings = ExportSettings {
        grid_only: true,
        ..prepare()
    };
    assert_eq!(
        plan_with(&record(44_100, 101_430), &flac, &settings),
        ExportOutcome::XmlOnly {
            reason: XmlOnlyReason::GridOnlyFlac
        }
    );
    // With a gain, FLAC is written.
    let plan = written(plan_with(&record(44_100, 101_430), &flac, &prepare()));
    assert_eq!(plan.bext, None);
}

#[test]
fn float_grid_only_is_xml_only() {
    let settings = ExportSettings {
        grid_only: true,
        ..prepare()
    };
    let float = ExportSource {
        float: true,
        bits_per_sample: Some(32),
        ..wav()
    };
    assert_eq!(
        plan_with(&record(44_100, 0), &float, &settings),
        ExportOutcome::XmlOnly {
            reason: XmlOnlyReason::GridOnlyWouldRequantise {
                float: true,
                bits: Some(32)
            }
        }
    );
    // Only 16- and 24-bit integers are written back at their own depth.
    for bits in [Some(8), Some(20), Some(32), None] {
        let int = ExportSource {
            bits_per_sample: bits,
            ..wav()
        };
        assert_eq!(
            plan_with(&record(44_100, 0), &int, &settings),
            ExportOutcome::XmlOnly {
                reason: XmlOnlyReason::GridOnlyWouldRequantise { float: false, bits }
            },
            "{bits:?}"
        );
    }
    let sixteen = ExportSource {
        bits_per_sample: Some(16),
        ..wav()
    };
    assert!(matches!(
        plan_with(&record(44_100, 0), &sixteen, &settings),
        ExportOutcome::Write { .. }
    ));
    let untagged = ExportSource {
        has_tag: false,
        ..wav()
    };
    assert_eq!(
        plan_with(&record(44_100, 0), &untagged, &settings),
        ExportOutcome::XmlOnly {
            reason: XmlOnlyReason::NoTagToWriteGridOnly
        }
    );
    // A float source with a gain is written (at 24 bits, the writer's default for float).
    assert!(matches!(
        plan_with(&record(44_100, 0), &float, &prepare()),
        ExportOutcome::Write { .. }
    ));
}

#[test]
fn mp3_is_xml_only() {
    for codec in [Codec::Mp3, Codec::Aac] {
        let source = ExportSource {
            codec,
            ..ExportSource::default()
        };
        for grid_only in [false, true] {
            let settings = ExportSettings {
                grid_only,
                ..prepare()
            };
            assert_eq!(
                plan_with(&record(44_100, 0), &source, &settings),
                ExportOutcome::XmlOnly {
                    reason: XmlOnlyReason::Mp3OrAac { codec }
                }
            );
        }
    }
    let alac = ExportSource {
        codec: Codec::Alac,
        ..ExportSource::default()
    };
    assert_eq!(
        plan_with(&record(44_100, 0), &alac, &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::Unsupported { codec: Codec::Alac }
        }
    );
}

#[test]
fn serato_blocks_in_place_cut() {
    let serato = ExportSource {
        serato: true,
        ..wav()
    };
    let r = record(44_100, 101_430);
    assert_eq!(
        plan_with(&r, &serato, &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::SeratoInPlaceCut {
                cut_s: SampleIndex(13_009).to_seconds(44_100)
            }
        }
    );
    // A copy in a folder may be cut: the original and its cue points stay as they are, and the
    // copy's Serato cues, carried byte for byte, are flagged as shifted by the cut.
    let folder = ExportSettings {
        place: Place::Folder,
        ..prepare()
    };
    let copy = written(plan_with(&r, &serato, &folder));
    assert_eq!(copy.trim_frames, 13_009);
    assert_eq!(
        copy.notices,
        [ExportNotice::SeratoCuesShifted {
            cut_s: SampleIndex(13_009).to_seconds(44_100)
        }]
    );
    assert_eq!(
        written(plan_with(&r, &wav(), &folder)).notices,
        Vec::<ExportNotice>::new()
    );
    // In place without a cut moves no position.
    let library = ExportSettings::new(BatchMode::Library);
    assert!(matches!(
        plan_with(&r, &serato, &library),
        ExportOutcome::Write { .. }
    ));
    let on_bar = written(plan_with(&record(48_000, 240), &serato, &prepare()));
    assert_eq!(on_bar.notices, Vec::<ExportNotice>::new());
}

#[test]
fn replaygain_values() {
    // I = -9.0, g = -2.0: the export reads -11 LUFS, 7 dB above the -18 LUFS reference; the
    // true peak -1 dBTP becomes -3 dBTP, 10^(-3/20) = 0.707946 linear.
    let plan = written(plan_with(&record(44_100, 0), &wav(), &prepare()));
    assert_eq!(tag(&plan, "REPLAYGAIN_TRACK_GAIN"), Some("-7.00 dB"));
    assert_eq!(tag(&plan, "REPLAYGAIN_TRACK_PEAK"), Some("0.707946"));
    assert_eq!(
        replaygain(Lufs(-20.0), DbTp(-0.5), 0.0),
        ("+2.00 dB".to_owned(), "0.944061".to_owned())
    );
    assert_eq!(replaygain(Lufs(-18.0), DbTp(0.0), 0.0).0, "+0.00 dB");
    // The bext loudness moves with the gain; the range does not.
    let bext = plan.bext.expect("bext for WAV");
    assert_eq!(
        (
            bext.integrated_lufs_x100,
            bext.range_lu_x100,
            bext.max_true_peak_dbtp_x100,
            bext.max_momentary_lufs_x100,
            bext.max_short_term_lufs_x100
        ),
        (-1100, 400, -300, -800, -900)
    );
}

#[test]
fn soundcheck_record_format() {
    let plan = written(plan_with(&record(44_100, 101_430), &wav(), &prepare()));
    assert_eq!(
        tag(&plan, "SOUNDCHECK"),
        Some(
            format!(
                "v=1;app={VERSION};mode=prepare;stat=S-P95;target=-11.00;gain=-2.00;\
                 trim=13009;rate=44100;bpm=120.00;bar1=221"
            )
            .as_str()
        )
    );
    let names: Vec<&str> = plan.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "BPM",
            "REPLAYGAIN_TRACK_GAIN",
            "REPLAYGAIN_TRACK_PEAK",
            "SOUNDCHECK"
        ]
    );
    // The source hash, when known, is named by its first 8 bytes.
    let hashed = ExportSource {
        blake3: Some([0xab; 32]),
        ..wav()
    };
    let plan = written(plan_with(&record(44_100, 101_430), &hashed, &prepare()));
    assert!(
        tag(&plan, "SOUNDCHECK")
            .expect("record")
            .ends_with(";bar1=221;src=abababababababab")
    );
}

#[test]
fn tbpm_off_in_library_by_default() {
    let library = ExportSettings::new(BatchMode::Library);
    let plan = written(plan_with(&record(44_100, 0), &wav(), &library));
    assert_eq!(tag(&plan, "BPM"), None);
    // The record still carries the tempo.
    assert!(
        tag(&plan, "SOUNDCHECK")
            .expect("record")
            .contains(";bpm=120.00;")
    );
    let on = ExportSettings {
        tbpm: true,
        ..library
    };
    let plan = written(plan_with(&record(44_100, 0), &wav(), &on));
    assert_eq!(tag(&plan, "BPM"), Some("120.00"));
}

#[test]
fn not_dj_safe_rates_silence_and_missing_grids() {
    assert_eq!(
        plan_with(&record(96_000, 0), &wav(), &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::NotDjSafeRate {
                sample_rate_hz: 96_000
            }
        }
    );
    let mut silent = record(44_100, 0);
    silent.loudness.short_term_p95 = None;
    silent.loudness.integrated = None;
    assert_eq!(
        plan_with(&silent, &wav(), &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::Silent
        }
    );
    let mut no_grid = record(44_100, 0);
    no_grid.grid = None;
    let plan = written(plan_with(&no_grid, &wav(), &prepare()));
    assert_eq!((plan.cut, plan.trim_frames), (Cut::NoGrid, 0));
    assert_eq!(tag(&plan, "BPM"), None);
    let record = tag(&plan, "SOUNDCHECK").expect("record");
    assert!(
        !record.contains("bpm=") && !record.contains("bar1="),
        "{record}"
    );
    let grid_only = ExportSettings {
        grid_only: true,
        ..prepare()
    };
    assert_eq!(
        plan_with(&no_grid, &wav(), &grid_only),
        ExportOutcome::Skip {
            reason: ExportSkip::NoGrid
        }
    );
}

#[test]
fn a_capped_boost_keeps_the_decided_gain() {
    // 9 dB wanted, 2.5 dB of headroom under the -0.5 dBTP ceiling: the plan's gain is taken.
    let mut r = record(44_100, 0);
    r.loudness.short_term_p95 = Some(Lufs(-20.0));
    r.loudness.true_peak = DbTp(-3.0);
    let plan = written(plan_with(&r, &wav(), &prepare()));
    assert!((plan.gain_db - 2.5).abs() < 1e-12, "{}", plan.gain_db);
    let aiff = ExportSource {
        codec: Codec::Aiff,
        ..wav()
    };
    let plan = written(plan_with(
        &r,
        &aiff,
        &ExportSettings {
            depth: Some(16),
            ..prepare()
        },
    ));
    assert_eq!((plan.bits, plan.bext), (Some(16), None));
}

#[test]
fn plans_are_deterministic() {
    let r = record(44_100, 101_430);
    let a = plan_with(&r, &wav(), &prepare());
    let b = plan_with(&r, &wav(), &prepare());
    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_string(&a).expect("json"),
        serde_json::to_string(&b).expect("json")
    );
}

#[test]
fn xml_off_leaves_nothing_to_write() {
    let no_xml = ExportSettings {
        xml: false,
        ..prepare()
    };
    let mp3 = ExportSource {
        codec: Codec::Mp3,
        ..ExportSource::default()
    };
    assert_eq!(
        plan_with(&record(44_100, 0), &mp3, &no_xml),
        ExportOutcome::Skip {
            reason: ExportSkip::NothingToWrite {
                reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 }
            }
        }
    );
    let flac = ExportSource {
        codec: Codec::Flac,
        ..wav()
    };
    let grid_only = ExportSettings {
        grid_only: true,
        ..no_xml
    };
    assert_eq!(
        plan_with(&record(44_100, 0), &flac, &grid_only),
        ExportOutcome::Skip {
            reason: ExportSkip::NothingToWrite {
                reason: XmlOnlyReason::GridOnlyFlac
            }
        }
    );
    // A file that is written does not need the XML.
    assert!(matches!(
        plan_with(&record(44_100, 0), &wav(), &no_xml),
        ExportOutcome::Write { .. }
    ));
}

#[test]
fn more_than_two_channels_are_skipped() {
    let mut r = record(48_000, 240);
    r.spec.channels = 6;
    assert_eq!(
        plan_with(&r, &wav(), &prepare()),
        ExportOutcome::Skip {
            reason: ExportSkip::UnsupportedChannels { channels: 6 }
        }
    );
    r.spec.channels = 1;
    assert!(matches!(
        plan_with(&r, &wav(), &prepare()),
        ExportOutcome::Write { .. }
    ));
}
