//! The self-check of exported grids (`sc_engine::export::grid_check`) on synthetic kick-and-click
//! tracks with an accented bar (the grid tests need the model files and are skipped without): a
//! Prepare export checks itself and records the result in the event, the sidecar, the journal
//! and the grid report; the user's own choices (a nudged bar 1, a typed tempo, another beat 1)
//! pass; audio that lost 20 ms or a beat more than the export recorded fails, also when the user
//! chose beat 1; a bar-1 kick 5 ms into a file is found there. Files without a grid are reported
//! as not checked.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // test arithmetic on small counts and sample positions

mod common;

use std::path::{Path, PathBuf};

use common::export::*;
use sc_core::Seconds;
use sc_core::analysis::{AnalysisRecord, Grid, GridEdit};
use sc_core::export::{
    BatchMode, ExportOutcome, GridCheck, GridCheckSkip, XmlGrid, XmlOnlyReason, XmlTrackInfo,
};
use sc_core::export::{ExportRecord, ExportedGrid};
use sc_core::plan::Codec;
use sc_engine::export::grid_check::{Choices, compare, detector_grid};
use sc_engine::export::report_rows;
use sc_engine::{Analyzer, BatchRow, CancelToken, XmlSelect};

const RATE: u32 = 44_100;

/// `lead_s` of a quiet noise floor (-50 dBFS), then `beats` beats at `bpm`: on each a 60 Hz kick
/// (80 ms, decaying) and a 1 kHz click (15 ms), both far louder on the first beat of every four,
/// so bar 1 is unambiguous; 16-bit stereo 44.1 kHz.
fn kick_track(dir: &Path, name: &str, bpm: f64, beats: u32, lead_s: f64) -> PathBuf {
    kick_track_with(dir, name, bpm, beats, lead_s, ACCENTS)
}

/// Kick and click levels on bar 1 and on the other beats. With these the beat tracker puts bar 1
/// on the accented beat wherever the track starts; a much stronger accent makes it read the
/// accented beat as the last of the bar (confidently), a weaker one leaves the bar to chance.
const ACCENTS: [f64; 4] = [0.6, 0.3, 0.5, 0.35];

/// [`kick_track`] with the accents `a`: kick on bar 1, kick elsewhere, click on bar 1, click
/// elsewhere.
fn kick_track_with(
    dir: &Path,
    name: &str,
    bpm: f64,
    beats: u32,
    lead_s: f64,
    a: [f64; 4],
) -> PathBuf {
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
        let tick = if i % 4 == 0 { a[2] } else { a[3] };
        let accent = if i % 4 == 0 { a[0] } else { a[1] };
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
fn analysed(path: &Path) -> AnalysisRecord {
    Analyzer::load(common::with_grid(), None, CancelToken::new())
        .expect("model")
        .analyze(path)
        .expect("analysed")
        .record
}

const BPM: f64 = 124.0;

/// One beat at [`BPM`], frames.
const BEAT_FRAMES: usize = 21_339;

/// Exports `path` in Prepare mode in place after saving `edit` of its analysed grid, confirmed
/// (a synthetic track's downbeat margin keeps its confidence below green, and a grid that needs
/// review is not cut); its `Done` payload, the sidecar's export record and the source's grid.
fn export_with(
    lib: &Lib,
    path: &Path,
    edit: impl Fn(&Grid) -> GridEdit,
) -> (sc_engine::ProcessDone, ExportRecord, Grid) {
    let analysis = common::with_grid();
    let record = Analyzer::load(analysis.clone(), Some(lib.cache()), CancelToken::new())
        .expect("model")
        .analyze(path)
        .expect("analysed")
        .record;
    let grid = record.grid.clone().expect("grid");
    sc_engine::save_edit(
        &lib.edits(),
        &record,
        analysis.bpm_range,
        &edit(&grid),
        true,
    )
    .expect("confirmed");
    let s = settings(lib, analysis, process(lib, BatchMode::Prepare, None));
    let events = run(&batch(&[path.to_path_buf()]), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let done = done_of(&events).clone();
    let export = sidecar_of(path).export.expect("export recorded");
    (done, export, grid)
}

/// A fresh kick track with bar 1 0.4 s in, in `lib`.
fn track(lib: &Lib) -> PathBuf {
    kick_track(&lib.music(), "kick.wav", BPM, 96, 0.4)
}

/// The check of `worse` (a further cut copy of an export) against `exported`, solved with the
/// choices the engine uses for a grid the user edited (`edited`) or not.
fn recheck(worse: &Path, exported: &ExportedGrid, edited: bool) -> GridCheck {
    let record = analysed(worse);
    let choices = Choices {
        edit: if edited {
            GridEdit {
                meter: Some(exported.meter.clone()),
                tempo_hint: Some(exported.bpm_exact),
                ..GridEdit::default()
            }
        } else {
            GridEdit::default()
        },
        bpm_range: common::with_grid().bpm_range,
    };
    compare(exported, detector_grid(&record, &choices).as_ref(), RATE)
}

fn offset(check: &GridCheck) -> f64 {
    match check {
        GridCheck::Pass { offset_ms, .. } | GridCheck::OffBy { offset_ms, .. } => *offset_ms,
        other => panic!("no offset: {other:?}"),
    }
}

/// Milliseconds of `x` samples.
fn ms(x: f64) -> f64 {
    x * 1000.0 / f64::from(RATE)
}

#[test]
fn prepared_click_track_passes() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = track(&lib);
    let (done, export, source) = export_with(&lib, &path, |_| GridEdit::default());
    // The analysis found the accented bar: bar 1 at the first accented kick (0.4 s), modulo
    // the bar, within 2 ms.
    let bar_ms = 4.0 * 60_000.0 / BPM;
    let bar1_ms = ms(source.anchor.0 as f64);
    let phase = (bar1_ms - 400.0).rem_euclid(bar_ms);
    assert!(phase.min(bar_ms - phase) <= 2.0, "bar 1 at {bar1_ms:.2} ms");
    assert!(
        export.plan.trim_frames > 0,
        "the head is cut: {:?}",
        export.plan.cut
    );
    let check = done.grid_check.clone();
    assert!(check.passed(), "{check:?}; notes {:?}", done.notes);
    assert!(offset(&check).abs() <= 2.0, "{check:?}");
    match &check {
        GridCheck::Pass { bpm_diff, .. } => assert!(bpm_diff.abs() <= 0.005, "{check:?}"),
        other => panic!("{other:?}"),
    }
    // The source's detector is the exported grid itself (no edit).
    let grid = export.grid.clone().expect("grid exported");
    assert!(
        grid.detector_offset_ms.is_some_and(|o| o.abs() < 1e-6),
        "{grid:?}"
    );
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
    assert!(report.starts_with("pass (bar line "), "{report}");
}

/// The grid report's `grid_check` cell of a row built from `done`.
fn done_report(path: &Path, done: &sc_engine::ProcessDone, export: &ExportRecord) -> String {
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
    let path = track(&lib);
    let (done, export, _) = export_with(&lib, &path, |_| GridEdit::default());
    assert!(done.grid_check.passed(), "{:?}", done.grid_check);
    let grid = export.grid.expect("grid exported");
    // The same audio with 20 ms (882 frames) more cut than the export recorded: bar 1 now sits
    // 20 ms before where the record puts it.
    let worse = lib.dir.path().join("worse.wav");
    cut_more(&path, 882, &worse);
    let check = recheck(&worse, &grid, false);
    assert!(matches!(check, GridCheck::OffBy { .. }), "{check:?}");
    assert!((offset(&check) + 20.0).abs() <= 2.0, "{check:?}");
    assert!(check.to_string().starts_with("off by -"), "{check}");
    // A whole beat more: a beat off, modulo the bar.
    let beat = lib.dir.path().join("beat.wav");
    cut_more(&path, BEAT_FRAMES, &beat);
    let check = recheck(&beat, &grid, false);
    let beat_ms = 60_000.0 / BPM;
    assert!((offset(&check).abs() - beat_ms).abs() <= 2.0, "{check:?}");
}

#[test]
fn a_nudged_bar_1_passes() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = track(&lib);
    // The user confirmed bar 1 12 ms (529 samples) after the analysed one.
    let (done, export, _) = export_with(&lib, &path, |g| GridEdit {
        anchor: Some(sc_core::SampleIndex(g.anchor.0 + 529)),
        ..GridEdit::default()
    });
    let grid = export.grid.expect("grid exported");
    let recorded = grid.detector_offset_ms.expect("detector recorded");
    assert!((recorded + 12.0).abs() <= 0.1, "{grid:?}");
    assert!(done.grid_check.passed(), "{:?}", done.grid_check);
    assert!(
        offset(&done.grid_check).abs() <= 2.0,
        "{:?}",
        done.grid_check
    );
}

#[test]
fn a_typed_tempo_passes() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = track(&lib);
    let (done, export, source) = export_with(&lib, &path, |g| GridEdit {
        bpm: Some(sc_core::Bpm(g.bpm.0 + 0.02)),
        ..GridEdit::default()
    });
    let grid = export.grid.expect("grid exported");
    assert!(
        (grid.bpm_exact.0 - source.bpm.0 - 0.02).abs() < 1e-9,
        "{grid:?}"
    );
    assert!(done.grid_check.passed(), "{:?}", done.grid_check);
}

#[test]
fn a_chosen_beat_1_passes_and_an_extra_beat_cut_still_fails() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = track(&lib);
    // The user makes the analysed beat 2 the first beat of the bar.
    let (done, export, _) = export_with(&lib, &path, |_| GridEdit {
        downbeat_shift: 1,
        ..GridEdit::default()
    });
    let grid = export.grid.expect("grid exported");
    let beat_ms = 60_000.0 / BPM;
    let recorded = grid.detector_offset_ms.expect("detector recorded");
    assert!((recorded.abs() - beat_ms).abs() <= 1.0, "{grid:?}");
    assert!(done.grid_check.passed(), "{:?}", done.grid_check);
    // The written audio with one beat more cut: a beat off, modulo the bar.
    let beat = lib.dir.path().join("beat.wav");
    cut_more(&path, BEAT_FRAMES, &beat);
    let check = recheck(&beat, &grid, true);
    assert!(matches!(check, GridCheck::OffBy { .. }), "{check:?}");
    assert!((offset(&check).abs() - beat_ms).abs() <= 2.0, "{check:?}");
}

#[test]
fn bar1_at_5ms_detected() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    // A file that starts 5 ms before its bar-1 kick, as a Prepare cut leaves it.
    let path = kick_track(&lib.music(), "cut.wav", BPM, 96, 0.005);
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
    // Bar 1 on the accented kick at 5 ms, modulo the bar.
    let bar_ms = 4.0 * 60_000.0 / grid.bpm.0;
    let anchor_ms = ms(grid.anchor.0 as f64);
    let phase = (anchor_ms - 5.0).rem_euclid(bar_ms);
    assert!(
        phase.min(bar_ms - phase) <= 2.0,
        "bar 1 at {anchor_ms:.2} ms, {phase:.2} ms off the bar"
    );
    assert!((grid.bpm.0 - BPM).abs() <= 0.02, "{}", grid.bpm.0);
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
