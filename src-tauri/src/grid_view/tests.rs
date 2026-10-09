//! The grid view's command bodies on a click track: open (analysis, decoding, player loaded),
//! waveform bins, onsets, refits with and without an edit, saving a confirmed edit, closing, and
//! the errors for a row that is not open, the version heard and the monitor volume, and the
//! player event built from the player's status. Needs the model files (skipped without); no test
//! starts playback, so no audio device is opened.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use sc_core::analysis::{AnalysisSettings, GridEdit, Model};
use sc_core::ipc::{
    AnalyzeRequest, GridFitHeader, IpcErrorKind, JobEvent, JobStage, Listen, MeterFrame,
    TrackEvent, Version,
};
use sc_core::plan::DecideSettings;
use sc_core::{AudioSpec, Bpm, DbFs, DbTp, Lufs, SampleIndex, testsig};
use sc_engine::player::{PlayerState, Status};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;

use super::player_event;
use crate::shell::Shell;

fn have_models() -> bool {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../models");
    let ok =
        dir.join("mel_spectrogram.onnx").is_file() && dir.join("beat_this_small.onnx").is_file();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok
}

/// 0.4 s of silence, then 72 clicks at 124 BPM, 16-bit stereo 44.1 kHz.
fn click_wav(dir: &Path) -> PathBuf {
    let path = dir.join("click.wav");
    let clicks = testsig::click_track(AudioSpec::CD, Bpm(124.0), 72, 15.0);
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..2 * testsig::frames_for(AudioSpec::CD, 0.4) {
        w.write_sample(0_i16).unwrap();
    }
    for s in &clicks.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * 0.5 * 32_767.0).round() as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    path
}

/// A shell with one analysed click track; returns it with the row's id.
fn analysed_shell(dir: &Path) -> (Shell, u32) {
    let wav = click_wav(dir);
    let shell = Shell::new(
        Some(Cache::open(dir.join("cache"))),
        Some(EditStore::open(dir.join("edits"))),
        1,
    );
    let rows = shell.expand(vec![wav.display().to_string()]);
    let (tx, rx) = mpsc::channel();
    shell
        .start(
            AnalyzeRequest {
                file_ids: vec![rows[0].file_id],
                analysis: AnalysisSettings {
                    bpm_range: (Bpm(70.0), Bpm(180.0)),
                    grid: true,
                    model: Model::Small,
                },
            },
            move |e| {
                let _ = tx.send(e);
            },
        )
        .unwrap();
    while !matches!(
        rx.recv_timeout(Duration::from_secs(60)).unwrap(),
        JobEvent::Finished { .. }
    ) {}
    (shell, rows[0].file_id)
}

/// Splits `grid_refit`'s bytes into the header and the residuals.
fn parse_fit(bytes: &[u8]) -> (GridFitHeader, Vec<f32>) {
    let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let header: GridFitHeader = serde_json::from_slice(&bytes[4..4 + len]).unwrap();
    let residuals = bytes[4 + len..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    (header, residuals)
}

#[test]
fn a_row_opens_refits_saves_a_confirmed_edit_and_closes() {
    if !have_models() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (shell, id) = analysed_shell(dir.path());
    let (tx, rx) = mpsc::channel();
    let opened = shell
        .open_track(id, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
    let grid = opened.grid.clone().expect("a grid");
    assert!((grid.bpm.0 - 124.0).abs() < 0.02, "{}", grid.bpm.0);
    assert_eq!(opened.analysed, opened.grid);
    assert_eq!(opened.sample_rate, 44_100);
    assert!(!opened.confirmed && opened.edit.is_empty());
    assert_eq!(opened.cover, None);
    // Decoding ends with `ready`; the cached analysis had its evidence, so no `analysing`.
    let ready = loop {
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            TrackEvent::Ready { frames } => break frames,
            TrackEvent::Decoded { .. } => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(ready, opened.frames);

    // 100 bins of 1,024 frames: 400 bytes of min/max pairs.
    assert_eq!(shell.peaks(id, 1024, 0, 100).unwrap().len(), 400);
    assert_ne!(
        shell.onsets(id).unwrap().len(),
        0,
        "the kick onsets are sent"
    );
    assert_eq!(shell.cover(id).unwrap(), Vec::<u8>::new());

    let (fit, residuals) = parse_fit(&shell.refit(id, &GridEdit::default()).unwrap());
    assert_eq!(
        fit.grid.as_ref(),
        Some(&grid),
        "an empty edit is the analysis"
    );
    assert_eq!(residuals.len(), fit.lines as usize);
    assert!(fit.matched > 50, "{fit:?}");
    let doubled = GridEdit {
        octave: 1,
        ..GridEdit::default()
    };
    let (fit, _) = parse_fit(&shell.refit(id, &doubled).unwrap());
    assert!((fit.grid.unwrap().bpm.0 - 248.0).abs() < 0.05);
    let too_fast = GridEdit {
        octave: 9,
        ..GridEdit::default()
    };
    assert_eq!(
        shell.refit(id, &too_fast).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );

    let update = shell.commit(id, &doubled, true).unwrap();
    assert!(update.row.edited && update.row.confirmed);
    assert_eq!(update.plan.status, JobStage::Analysed);
    let row_grid = update.row.grid.unwrap();
    assert!((row_grid.bpm.0 - 248.0).abs() < 0.05);

    shell.close_track();
    assert_eq!(
        shell.peaks(id, 1024, 0, 10).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );

    // Opened again, the saved edit and its confirmation come back.
    let reopened = shell.open_track(id, |_| {}).unwrap();
    assert!(reopened.confirmed);
    assert_eq!(reopened.edit, doubled);
    assert!((reopened.grid.unwrap().bpm.0 - 248.0).abs() < 0.05);
    assert!((reopened.analysed.unwrap().bpm.0 - 124.0).abs() < 0.05);
}

#[test]
fn a_row_that_is_not_open_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let shell = Shell::new(None, None, 1);
    assert_eq!(
        shell.open_track(7, |_| {}).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
    assert_eq!(
        shell.peaks(7, 1024, 0, 1).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
    assert_eq!(
        shell.refit(7, &GridEdit::default()).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
    assert_eq!(
        shell
            .commit(7, &GridEdit::default(), true)
            .unwrap_err()
            .kind,
        IpcErrorKind::InvalidArgument,
        "no place to save edits"
    );
    drop(dir);
}

/// Waits (up to 2 s) for the player's published status to satisfy `ok`.
fn player_status(shell: &Shell, ok: impl Fn(&Status) -> bool) -> Status {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let status = shell.player().as_ref().map(sc_engine::player::Player::status);
        if let Some(status) = status
            && (ok(&status) || std::time::Instant::now() > deadline)
        {
            return status;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_version_heard_follows_the_view_and_a_new_track_starts_processed() {
    if !have_models() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (shell, id) = analysed_shell(dir.path());
    let _ = shell.open_track(id, |_| {}).unwrap();
    let original = Listen {
        version: Version::Original,
        matched: true,
    };
    shell.set_listen(id, original).unwrap();
    let status = player_status(&shell, |s| s.listen == original);
    assert_eq!(status.listen, original);
    assert_eq!(status.meter, None, "nothing plays, nothing is metered");
    assert_eq!(
        shell.set_listen(id + 1, original).unwrap_err().kind,
        IpcErrorKind::InvalidArgument,
        "only the open track"
    );
    // A new target replans the rows and the open track's player follows (no lock is held twice).
    let mut settings = DecideSettings::dj();
    settings.target = Lufs(settings.target.0 - 3.0);
    let replan = shell.set_decide_settings(settings).unwrap();
    assert_eq!(replan.plans.len(), 1);

    let _ = shell.open_track(id, |_| {}).unwrap();
    let status = player_status(&shell, |s| s.listen == Listen::default());
    assert_eq!(status.listen, Listen::default(), "a load starts processed");
}

#[test]
fn the_volume_goes_up_to_0_db_and_needs_no_open_track() {
    let shell = Shell::new(None, None, 1);
    shell.set_volume(Some(DbFs(-12.5))).unwrap();
    shell.set_volume(None).unwrap();
    shell.set_volume(Some(DbFs(0.0))).unwrap();
    for wrong in [0.5, f64::NAN] {
        assert_eq!(
            shell.set_volume(Some(DbFs(wrong))).unwrap_err().kind,
            IpcErrorKind::InvalidArgument,
            "{wrong}"
        );
    }
    assert!(shell.player().is_some(), "the player started for it");
}

#[test]
fn the_player_event_carries_the_heard_reading_while_playing() {
    let frame = MeterFrame {
        position: SampleIndex(44_544),
        in_peak: Some(DbTp(-1.0)),
        in_momentary: Some(Lufs(-9.0)),
        out_peak: Some(DbTp(-3.5)),
        out_momentary: Some(Lufs(-11.5)),
    };
    let status = |playing| Status {
        state: Some(PlayerState {
            playing,
            position: SampleIndex(44_000),
            underruns: 0,
        }),
        meter: Some(frame),
        listen: Listen {
            version: Version::Original,
            matched: false,
        },
        ..Status::default()
    };
    let Some(TrackEvent::Player {
        meter,
        listen,
        position,
        ..
    }) = player_event(&status(true), false)
    else {
        panic!("an event while playing");
    };
    assert_eq!(meter, Some(frame));
    assert!(meter.unwrap().position.0.abs_diff(position.0) <= 1_024);
    assert_eq!(listen.version, Version::Original);
    let Some(TrackEvent::Player { meter, playing, .. }) = player_event(&status(false), true)
    else {
        panic!("one event when playing stops");
    };
    assert!(!playing);
    assert_eq!(meter, None, "stopped: empty meters");
    assert_eq!(player_event(&status(false), false), None, "then nothing");
    assert_eq!(player_event(&Status::default(), true), None, "no track");
}
