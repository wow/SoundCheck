//! The grid view's fit choice: offered for a track whose tempo changes (its whole-track grid
//! drifts, or holds far fewer of the opening kicks than the start fit) or when the edit already
//! fits the start, with the start window's kick shares of both fits; not offered for a steady
//! track or without attacks at the start. A confirmed start fit survives the edit store.
//! Synthetic evidence shaped like the model's output.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // test arithmetic

use sc_core::Bpm;
use sc_core::analysis::{AnalysisRecord, GridEdit, GridEvidence, GridFit, OnsetList, Verdict};
use sc_engine::edits::refit_record;
use sc_engine::{apply_saved, fit_choice, save_edit};
use sc_io::edits::EditStore;

const ANALYSIS_RATE: u32 = 22_050;
const RANGE: (Bpm, Bpm) = (Bpm(70.0), Bpm(180.0));

/// A 4/4 track with a kick on every beat from 0.5 s: `bpm_a` until `change_s`, then `bpm_b`
/// until `end_s`; beats on the model's 20 ms frames, downbeats on every fourth beat.
fn evidence(bpm_a: f64, change_s: f64, bpm_b: f64, end_s: f64) -> GridEvidence {
    let mut times = Vec::new();
    let mut t = 0.5;
    while t < end_s {
        times.push(t);
        t += 60.0 / if t < change_s { bpm_a } else { bpm_b };
    }
    let kicks = OnsetList {
        sample_rate: ANALYSIS_RATE,
        frames: times
            .iter()
            .map(|t| ((t * 1000.0).round() / 1000.0 * f64::from(ANALYSIS_RATE)).round() as u32)
            .collect(),
        rise_db: vec![30.0; times.len()],
        level_db: (0..times.len())
            .map(|i| if i % 4 == 0 { 0.0 } else { -3.0 })
            .collect(),
    };
    let mut logits = vec![-6.0_f32; (end_s * 50.0).ceil() as usize + 2];
    let (mut beats_s, mut downbeats_s) = (Vec::new(), Vec::new());
    for (i, t) in times.iter().enumerate() {
        let frame = (t * 50.0).round();
        beats_s.push((frame / 50.0) as f32);
        if i % 4 == 0 {
            downbeats_s.push((frame / 50.0) as f32);
            logits[frame as usize] = 4.0;
        }
    }
    GridEvidence {
        beats_s,
        downbeats_s,
        downbeat_logits_50fps: logits,
        kick_onsets: kicks.clone(),
        broadband_onsets: kicks,
    }
}

fn record(evidence: GridEvidence) -> AnalysisRecord {
    let json = r#"{"schema":2,"version":"0","path":"/a.wav","size":1,"mtimeNs":2,
        "spec":{"sampleRate":44100,"channels":2},"frames":7938000,"duration":180.0,"delay":0,
        "padding":0,"loudness":{"integrated":null,"momentaryMax":null,"shortTermMax":null,
        "shortTermP95":null,"shortTermTop30":null,"lra":null,"truePeak":-1.0,
        "samplePeak":-1.0,"plr":null,"dualMono":false,"timeline":{"hopMs":100,
        "shortTerm":[]}},"grid":null,"gridSkipped":null,"tags":{"bpm":null,"genre":null,
        "title":null,"artist":null},"evidence":null}"#;
    let mut record: AnalysisRecord = serde_json::from_str(json).expect("a record");
    record.evidence = Some(evidence);
    record
}

#[test]
fn a_steady_track_is_not_offered_the_start_fit() {
    let steady = record(evidence(124.0, f64::INFINITY, 124.0, 180.0));
    let edit = GridEdit::default();
    let solved = refit_record(&steady, RANGE, &edit).expect("a grid");
    assert_eq!(solved.grid.verdict, Verdict::Static);
    assert_eq!(fit_choice(&steady, RANGE, &edit, Some(&solved)), None);
    assert_eq!(fit_choice(&steady, RANGE, &edit, None), None);
    // Asked for, it is shown even so: both fits hold every kick.
    let start = GridEdit {
        fit: GridFit::Start,
        ..GridEdit::default()
    };
    let choice = fit_choice(&steady, RANGE, &start, None).expect("a choice");
    assert!(
        choice.whole_share > 0.99 && choice.start_share > 0.99,
        "{choice:?}"
    );
}

#[test]
fn a_track_whose_tempo_changes_is_offered_the_start_fit() {
    let changing = record(evidence(116.0, 60.0, 117.2, 180.0));
    let whole = GridEdit::default();
    let solved = refit_record(&changing, RANGE, &whole).expect("a grid");
    assert_eq!(solved.grid.verdict, Verdict::Drifts);
    let choice = fit_choice(&changing, RANGE, &whole, Some(&solved)).expect("a choice");
    assert!(
        choice.start_share > choice.whole_share + 0.2 && choice.start_share > 0.85,
        "{choice:?}"
    );
    assert!((60.0..75.0).contains(&choice.window_end_s), "{choice:?}");
    // The same numbers from the start fit's side, with or without its grid at hand.
    let start = GridEdit {
        fit: GridFit::Start,
        ..GridEdit::default()
    };
    let start_solved = refit_record(&changing, RANGE, &start).expect("a grid");
    assert_eq!(
        fit_choice(&changing, RANGE, &start, Some(&start_solved)),
        Some(choice)
    );
    assert_eq!(fit_choice(&changing, RANGE, &start, None), Some(choice));
    assert_eq!(fit_choice(&changing, RANGE, &whole, None), Some(choice));
}

#[test]
fn no_evidence_gives_no_choice() {
    let mut bare = record(GridEvidence::default());
    bare.evidence = None;
    let edit = GridEdit {
        fit: GridFit::Start,
        ..GridEdit::default()
    };
    assert_eq!(fit_choice(&bare, RANGE, &edit, None), None);
}

/// `record` without attacks before `from_s`.
fn attacks_from(mut record: AnalysisRecord, from_s: f64) -> AnalysisRecord {
    let ev = record.evidence.as_mut().expect("evidence");
    for list in [&mut ev.kick_onsets, &mut ev.broadband_onsets] {
        let cut = (from_s * f64::from(ANALYSIS_RATE)) as u32;
        let keep = list.frames.iter().position(|&f| f >= cut).unwrap_or(0);
        list.frames.drain(..keep);
        list.rise_db.drain(..keep);
        list.level_db.drain(..keep);
    }
    record
}

#[test]
fn a_whole_track_grid_that_misses_the_start_offers_the_start_fit_without_drifting() {
    // A slight change after a minute of a six-minute track, and a large one after 30 s: the
    // whole-track grid matches too few of the opening attacks for its verdict to drift.
    for (a, change_s, b, end_s) in [(116.0, 60.0, 117.2, 360.0), (110.0, 30.0, 124.0, 180.0)] {
        let track = record(evidence(a, change_s, b, end_s));
        let whole = GridEdit::default();
        let solved = refit_record(&track, RANGE, &whole).expect("a grid");
        assert_ne!(solved.grid.verdict, Verdict::Drifts, "{a}->{b}");
        let choice = fit_choice(&track, RANGE, &whole, Some(&solved)).expect("a choice");
        assert!(
            choice.start_share >= choice.whole_share + 0.2,
            "{a}->{b}: {choice:?}"
        );
    }
}

#[test]
fn no_start_fit_without_attacks_at_the_start() {
    // About 29 attacks, all in the last 15 s.
    let late = attacks_from(record(evidence(116.0, 60.0, 117.2, 180.0)), 165.0);
    for fit in [GridFit::Whole, GridFit::Start] {
        let edit = GridEdit {
            fit,
            ..GridEdit::default()
        };
        assert_eq!(fit_choice(&late, RANGE, &edit, None), None, "{fit:?}");
    }
    // A start edit then gives the whole-track grid.
    let start = GridEdit {
        fit: GridFit::Start,
        ..GridEdit::default()
    };
    assert_eq!(
        refit_record(&late, RANGE, &start).map(|s| s.grid),
        refit_record(&late, RANGE, &GridEdit::default()).map(|s| s.grid)
    );
}

#[test]
fn a_confirmed_start_fit_survives_the_edit_store() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = EditStore::open(dir.path());
    let track = record(evidence(116.0, 60.0, 117.2, 180.0));
    let start = GridEdit {
        fit: GridFit::Start,
        ..GridEdit::default()
    };
    let expected = refit_record(&track, RANGE, &start).expect("a grid").grid;
    assert!((expected.bpm.0 - 116.0).abs() <= 0.01, "{}", expected.bpm.0);
    let saved = save_edit(&store, &track, RANGE, &start, true).expect("saved");
    assert!(saved.confirmed && saved.edited);
    assert_eq!(saved.fit, GridFit::Start);
    assert_eq!(store.get(&track.path).map(|s| s.edit), Some(start));

    let mut reopened = track.clone();
    let state = apply_saved(&mut reopened, &store);
    assert!(state.confirmed, "{state:?}");
    assert!(state.edited);
    assert_eq!(state.fit, GridFit::Start);
    assert_eq!(reopened.grid, Some(expected));
}
