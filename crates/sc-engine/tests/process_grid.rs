//! `run_batch` with `Task::Process` on a click track with the grid (needs the model files;
//! skipped without): a Prepare cut that puts bar 1 at the lead in the audio, and grid edits
//! carried over to the written file, confirmed or not, and back after undo.

mod common;

use common::export::*;
use sc_core::analysis::GridEdit;
use sc_core::export::{BatchMode, Cut};
use sc_engine::{Analyzer, CancelToken, apply_saved, save_edit};

#[test]
fn prepare_puts_bar_1_at_the_lead() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let frames_in = u64::from(frames(&path));
    let sound_in = first_sound(&path);
    let s = settings(
        &lib,
        common::with_grid(),
        process(&lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let doc = sidecar_of(&path);
    let export = doc.export.expect("export recorded");
    let trim = export.plan.trim_frames;
    assert!(matches!(export.plan.cut, Cut::Cut { frames, .. } if frames == trim));
    assert_eq!(doc.render.trim_frames(), trim);
    assert_eq!(u64::from(frames(&path)), frames_in - trim);
    // Bar 1 lands between the lead (5 ms: 220.5 samples) and 1 ms plus a sample after it.
    let grid = export.grid.expect("grid exported");
    // 5 ms is 220.5 samples at 44.1 kHz; 1 ms is 44.1: bar 1 lands at sample 220 or 221 (the
    // lead rounded) up to 265.
    let bar1 = grid.first_bar_line.0;
    assert!((220..=265).contains(&bar1), "{bar1}");
    assert_eq!(grid.bar1, grid.first_bar_line, "bar 1 is the first line");
    assert_eq!(grid.bpm.0, 120.0);
    // The audio moved by exactly the cut.
    assert_eq!(first_sound(&path), sound_in - trim);
}

#[test]
fn confirmed_edit_survives_export() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let analysis = common::with_grid();
    let mut analyzer =
        Analyzer::load(analysis.clone(), Some(lib.cache()), CancelToken::new()).expect("model");
    let record = analyzer.analyze(&path).expect("analysed").record;
    let anchor = record.grid.as_ref().expect("grid").anchor;
    // The user confirms the analysed grid by ear (an empty edit, confirmed).
    let edit = GridEdit::default();
    save_edit(&lib.edits(), &record, analysis.bpm_range, &edit, true).expect("saved");
    let s = settings(
        &lib,
        analysis.clone(),
        process(&lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert_eq!(
        names(&events, 1),
        ["started", "processing", "written", "done"]
    );
    let done = done_of(&events);
    assert!(done.edit_carried && done.edit.confirmed, "{:?}", done.notes);
    let trim = sidecar_of(&path).export.expect("export").plan.trim_frames;
    assert!(trim > 0);
    let out_grid = done
        .analysis
        .as_ref()
        .and_then(|r| r.grid.clone())
        .expect("grid");
    let expected = anchor.0 - trim;
    assert!(
        out_grid.anchor.0.abs_diff(expected) <= 1,
        "{out_grid:?} vs {expected}"
    );

    // A later session finds the edit for the written file, still confirmed.
    let mut again = Analyzer::load(analysis, Some(lib.cache()), CancelToken::new())
        .expect("model")
        .analyze(&path)
        .expect("analysed")
        .record;
    let state = apply_saved(&mut again, &lib.edits());
    assert!(state.confirmed);
    assert!(again.grid.expect("grid").anchor.0.abs_diff(expected) <= 1);
}

/// The click track analysed with the grid into `lib`'s cache.
fn analysed(lib: &Lib, path: &std::path::Path) -> sc_core::analysis::AnalysisRecord {
    Analyzer::load(common::with_grid(), Some(lib.cache()), CancelToken::new())
        .expect("model")
        .analyze(path)
        .expect("analysed")
        .record
}

#[test]
fn a_confirmed_edit_comes_back_after_undo() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let record = analysed(&lib, &path);
    let grid = record.grid.clone().expect("grid");
    let range = common::with_grid().bpm_range;
    save_edit(&lib.edits(), &record, range, &GridEdit::default(), true).expect("saved");
    let s = settings(
        &lib,
        common::with_grid(),
        process(&lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert!(done_of(&events).edit.confirmed);
    sc_engine::undo_file(&path, &lib.backups()).expect("undone");
    let mut again = analysed(&lib, &path);
    let state = apply_saved(&mut again, &lib.edits());
    assert!(state.confirmed, "{state:?}");
    assert_eq!(again.grid.as_ref(), Some(&grid));
}

#[test]
fn a_folder_export_leaves_the_source_edit() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let record = analysed(&lib, &path);
    let range = common::with_grid().bpm_range;
    let edit = GridEdit {
        bpm: Some(sc_core::Bpm(120.0)),
        ..GridEdit::default()
    };
    save_edit(&lib.edits(), &record, range, &edit, true).expect("saved");
    let mut before = record.clone();
    let state_before = apply_saved(&mut before, &lib.edits());
    let out = lib.dir.path().join("out");
    let s = settings(
        &lib,
        common::with_grid(),
        process(&lib, BatchMode::Prepare, Some(out.clone())),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    assert!(done_of(&events).edit_carried);
    let mut after = analysed(&lib, &path);
    assert_eq!(apply_saved(&mut after, &lib.edits()), state_before);
    assert_eq!(after.grid, before.grid);
    // The copy has the edit too.
    let mut copy = analysed(&lib, &out.join(path.file_name().expect("name")));
    assert!(apply_saved(&mut copy, &lib.edits()).confirmed);
}

#[test]
fn an_unconfirmed_edit_keeps_its_overrides() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let record = analysed(&lib, &path);
    let range = common::with_grid().bpm_range;
    let edit = GridEdit {
        bpm: Some(sc_core::Bpm(120.01)),
        ..GridEdit::default()
    };
    let state = save_edit(&lib.edits(), &record, range, &edit, false).expect("saved");
    assert!(state.edited && !state.confirmed);
    let s = settings(
        &lib,
        common::with_grid(),
        process(&lib, BatchMode::Prepare, None),
    );
    let events = run(&batch(std::slice::from_ref(&path)), &s, &CancelToken::new());
    let done = done_of(&events);
    assert!(
        done.edit_carried && !done.edit.confirmed,
        "{:?}",
        done.notes
    );
    let out = done.analysis.as_ref().expect("analysed");
    let audio = sc_engine::edits::audio_of(out);
    let carried = lib.edits().get(&out.path, &audio).expect("carried");
    assert!(!carried.confirmed);
    // The typed tempo stays typed; nothing else is pinned.
    assert_eq!(carried.edit, edit);
    assert_eq!(out.grid.as_ref().map(|g| g.bpm), Some(sc_core::Bpm(120.01)));
}

#[test]
fn bar_1_inside_the_cut_is_refused() {
    if !common::have_models() {
        return;
    }
    let lib = Lib::new();
    let path = click(&lib);
    let record = analysed(&lib, &path);
    let grid = record.grid.clone().expect("grid");
    let range = common::with_grid().bpm_range;
    save_edit(&lib.edits(), &record, range, &GridEdit::default(), true).expect("saved");
    let saved = lib
        .edits()
        .get(&record.path, &sc_engine::edits::audio_of(&record))
        .expect("saved");
    let err = sc_engine::carry_edit(
        &lib.edits(),
        &saved,
        Some(&grid),
        true,
        grid.anchor.0 + 1,
        "/elsewhere/a.wav",
        sc_engine::edits::audio_of(&record),
    )
    .expect_err("refused");
    assert!(matches!(err, sc_core::Error::InvalidArgument(_)), "{err}");
}
