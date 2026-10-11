//! The self-check of exported grids (`sc_engine::export::grid_check`) on synthetic kick-and-click
//! tracks (the grid tests need the model files and are skipped without): a Prepare export checks
//! itself and records the result in the event, the sidecar, the journal and the grid report; a
//! file that lost 20 ms more than the export recorded is off by 20 ms; a bar-1 kick 5 ms into a
//! file is found there. Files without a grid are reported as not checked.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // test arithmetic on small counts and sample positions

mod common;

use std::path::{Path, PathBuf};

use common::export::*;
use sc_core::Seconds;
use sc_core::export::{
    BatchMode, CheckPeriod, ExportOutcome, GridCheck, GridCheckSkip, XmlGrid, XmlOnlyReason,
    XmlTrackInfo,
};
use sc_core::plan::Codec;
use sc_engine::export::grid_check::compare;
use sc_engine::export::report_rows;
use sc_engine::{Analyzer, BatchRow, CancelToken, XmlSelect};

const RATE: u32 = 44_100;

/// `lead_s` of a quiet noise floor, then `beats` beats at `bpm`: on each a 60 Hz kick (80 ms, decaying;
/// louder on the first beat of every four) and a 1 kHz click (10 ms); 16-bit stereo 44.1 kHz.
fn kick_track(dir: &Path, name: &str, bpm: f64, beats: u32, lead_s: f64) -> PathBuf {
    let path = dir.join(name);
    let rate = f64::from(RATE);
    let beat = 60.0 / bpm;
    let total = lead_s + beat * f64::from(beats) + 0.2;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (total * rate) as usize;
    // A quiet noise floor (-50 dBFS), as music has between its kicks.
    let floor = sc_core::testsig::seeded_noise(sc_core::AudioSpec::new(RATE, 1), 7, 0.003, total);
    let mut mono: Vec<f64> = floor.data.iter().map(|&x| f64::from(x)).collect();
    mono.resize(frames, 0.0);
    for i in 0..beats {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let start = ((lead_s + beat * f64::from(i)) * rate).round() as usize;
        // The click accents the bar as `testsig::click_track` does; the kick is the same on
        // every beat.
        let tick = if i % 4 == 0 { 0.5 } else { 0.35 };
        let accent = if i % 4 == 0 { 0.5 } else { 0.3 };
        for n in 0..(0.08 * rate) as usize {
            #[allow(clippy::cast_precision_loss)]
            let t = n as f64 / rate;
            let kick = accent * (std::f64::consts::TAU * 60.0 * t).sin() * (-t / 0.03).exp();
            let click = if t < 0.015 {
                tick * (std::f64::consts::TAU * 1000.0 * t).sin()
            } else {
                0.0
            };
            if let Some(s) = mono.get_mut(start + n) {
                *s += kick + click;
            }
        }
    }
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for s in mono {
        #[allow(clippy::cast_possible_truncation)]
        let q = (s * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

/// `path` without its first `frames` frames, written to `out`.
fn cut_more(path: &Path, frames: usize, out: &Path) {
    let mut reader = hound::WavReader::open(path).expect("wav");
    let spec = reader.spec();
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .map(|s| s.expect("sample"))
        .collect();
    let mut w = hound::WavWriter::create(out, spec).expect("create wav");
    for s in &samples[frames * usize::from(spec.channels)..] {
        w.write_sample(*s).expect("write");
    }
    w.finalize().expect("finalize");
}

/// The grid `path` analyses to (no cache), with its evidence.
fn analysed(path: &Path) -> sc_core::analysis::AnalysisRecord {
    Analyzer::load(common::with_grid(), None, CancelToken::new())
        .expect("model")
        .analyze(path)
        .expect("analysed")
        .record
}

/// Exports `path` in Prepare mode in place after confirming its analysed grid unchanged (a
/// synthetic track's downbeat margin keeps it below green, and a grid that needs review is not
/// cut); its `Done` payload and the sidecar's export record.
fn export_prepare(
    lib: &Lib,
    path: &Path,
) -> (sc_engine::ProcessDone, sc_core::export::ExportRecord) {
    let analysis = common::with_grid();
    let record = Analyzer::load(analysis.clone(), Some(lib.cache()), CancelToken::new())
        .expect("model")
        .analyze(path)
        .expect("analysed")
        .record;
    sc_engine::save_edit(
        &lib.edits(),
        &record,
        analysis.bpm_range,
        &sc_core::analysis::GridEdit::default(),
        true,
    )
    .expect("confirmed");
    let s = settings(
        lib,
        common::with_grid(),
        process(lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(&[path.to_path_buf()]), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let done = done_of(&events).clone();
    let export = sidecar_of(path).export.expect("export recorded");
    (done, export)
}

#[test]
fn prepared_click_track_passes() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = kick_track(&lib.music(), "kick.wav", 124.0, 96, 0.4);
    let (done, export) = export_prepare(&lib, &path);
    assert!(
        export.plan.trim_frames > 0,
        "the head is cut: {:?}",
        export.plan.cut
    );
    let check = done.grid_check.clone();
    match &check {
        GridCheck::Pass {
            offset_ms,
            bpm_diff,
            period,
        } => {
            assert!(offset_ms.abs() <= 2.0, "{check:?}");
            assert!(bpm_diff.abs() <= 0.005, "{check:?}");
            assert_eq!(*period, CheckPeriod::Bar);
        }
        other => panic!("expected a pass: {other:?}; notes {:?}", done.notes),
    }
    // Recorded in the sidecar and the journal, so a sidecar written again keeps it.
    assert_eq!(export.grid_check.as_ref(), Some(&check));
    let entries = sc_io::txn::journal_entries(&lib.backups()).expect("journal");
    let journaled = entries
        .iter()
        .rev()
        .find_map(|e| e.record.as_ref()?.export.as_ref()?.grid_check.clone());
    assert_eq!(journaled.as_ref(), Some(&check));
    // In the grid report's column, as `sc-cli process` builds its row.
    let report = done_report(&path, &done, &export);
    assert!(
        report.starts_with("pass (bar line +") || report.starts_with("pass (bar line -"),
        "{report}"
    );
}

/// The grid report's `grid_check` cell of a row built from `done`.
fn done_report(
    path: &Path,
    done: &sc_engine::ProcessDone,
    export: &sc_core::export::ExportRecord,
) -> String {
    let mut row = BatchRow::written(
        path,
        BatchMode::Prepare,
        &export.plan,
        &done.output,
        export.plan.expect_frames,
        RATE,
        done.xml.clone(),
    );
    row.grid_check = Some(done.grid_check.clone());
    report_rows(&[row], XmlSelect::Batch)[0]
        .grid_check
        .clone()
        .expect("checked")
}

#[test]
fn extra_20_ms_trim_fails() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = kick_track(&lib.music(), "kick.wav", 124.0, 96, 0.4);
    let (done, export) = export_prepare(&lib, &path);
    assert!(done.grid_check.passed(), "{:?}", done.grid_check);
    let grid = export.grid.expect("grid exported");
    // The same audio with 20 ms (882 frames) more cut than the export recorded: bar 1 now sits
    // 20 ms before where the record puts it.
    let worse = lib.dir.path().join("worse.wav");
    cut_more(&path, 882, &worse);
    let record = analysed(&worse);
    let check = compare(&grid, record.grid.as_ref(), RATE, CheckPeriod::Bar);
    match check {
        GridCheck::OffBy { offset_ms, .. } => {
            assert!((offset_ms + 20.0).abs() <= 2.0, "{check:?}");
        }
        other => panic!("expected off by about -20 ms: {other:?}"),
    }
    assert!(check.to_string().starts_with("off by -"), "{check}");
}

#[test]
fn bar1_at_5ms_detected() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    // A file that starts 5 ms before its bar-1 kick, as a Prepare cut leaves it.
    let path = kick_track(&lib.music(), "cut.wav", 124.0, 96, 0.005);
    let record = analysed(&path);
    let evidence = record.evidence.as_ref().expect("evidence");
    let kicks = &evidence.kick_onsets;
    let first = f64::from(*kicks.frames.first().expect("kick onsets")) * 1000.0
        / f64::from(kicks.sample_rate);
    assert!(
        (first - 5.0).abs() <= 2.0,
        "first kick onset at {first:.2} ms"
    );
    let grid = record.grid.as_ref().expect("grid");
    // A grid line at the kick: the anchor modulo the beat (this synthetic's downbeat margin
    // is low, so which beat is beat 1 is not what is tested here).
    let beat_ms = 60_000.0 / grid.bpm.0;
    let anchor_ms = grid.anchor.to_seconds(RATE).0 * 1000.0;
    let line_ms = 5.0
        + ((anchor_ms - 5.0) / beat_ms)
            .round()
            .mul_add(-beat_ms, anchor_ms - 5.0);
    assert!(
        (line_ms - 5.0).abs() <= 2.0,
        "nearest line at {line_ms:.2} ms (bar 1 {anchor_ms:.2})"
    );
    assert!((grid.bpm.0 - 124.0).abs() <= 0.02, "{}", grid.bpm.0);
}

#[test]
fn a_file_without_a_grid_is_not_checked() {
    // Loudness only (no model needed): a Library export writes the gain and checks nothing.
    let lib = Lib::new();
    let path = common::tone_wav(&lib.music(), "tone.wav", 12.0, 0.9);
    let s = settings(
        &lib,
        common::loudness_only(),
        process(&lib, BatchMode::Library, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let expected = GridCheck::NotChecked {
        reason: GridCheckSkip::NoGrid,
    };
    assert_eq!(done_of(&events).grid_check, expected);
    let export = sidecar_of(&path).export.expect("export");
    assert_eq!(export.grid_check, Some(expected.clone()));
    assert_eq!(
        done_report(&path, done_of(&events), &export),
        "not checked: no grid exported"
    );
}

#[test]
fn a_file_left_to_the_xml_is_not_checked() {
    let outcome = ExportOutcome::XmlOnly {
        reason: XmlOnlyReason::Mp3OrAac { codec: Codec::Mp3 },
    };
    let row = BatchRow::not_written(
        Path::new("/m/a.mp3"),
        BatchMode::Prepare,
        &outcome,
        Seconds(200.0),
        Codec::Mp3,
        XmlTrackInfo {
            grid: XmlGrid::Absent,
            title: None,
            artist: None,
        },
    );
    let cells = report_rows(&[row], XmlSelect::Batch);
    assert_eq!(
        cells[0].grid_check.as_deref(),
        Some("not checked: file not written (XML only)")
    );
}
