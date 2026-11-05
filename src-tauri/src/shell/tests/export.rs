//! The export command on synthetic tones, loudness only (no model): the round trip over the
//! channel, refusals before anything is written, and cancel.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use sc_core::export::{Cut, ExportSettings, ExportSkip, GridCheck, GridCheckSkip};
use sc_core::ipc::{
    ExportAction, ExportRequest, IpcErrorKind, JobEvent, JobStage, MAX_ROW_BYTES, RecoveryStatus,
    WriteXmlRequest,
};
use sc_io::txn;

use super::support::{
    Lib, analysed, events_until_finished, export, in_folder, no_temp_left, prepare, request,
    sha256, steps, tone_wav,
};
/// What the UI shows for the written file: the gain up to -11 LUFS S-P95, no grid to cut to, and
/// the verification; the row updated to the written file's own measurements.
fn written_row_shows_the_export(events: &[JobEvent]) {
    let (row, update) = events
        .iter()
        .find_map(|e| match e {
            JobEvent::ExportDone { row, update, .. } => Some((row, update)),
            _ => None,
        })
        .expect("done");
    let ExportAction::Write { gain_db, cut } = row.action else {
        panic!("{:?}", row.action);
    };
    assert!(gain_db > 0.5, "{gain_db}");
    assert_eq!(cut, Cut::NoGrid);
    assert!(!row.positions_shift && !row.grid_withheld && row.revertible);
    assert_eq!(
        row.grid_check,
        Some(GridCheck::NotChecked {
            reason: GridCheckSkip::NoGrid
        })
    );
    let v = row.verification.as_ref().expect("verified");
    assert_eq!(v.frames_in, v.frames_out);
    assert_eq!(v.frames_in, 5 * 44_100);
    assert_eq!(v.bits_out, 16);
    assert_eq!(
        v.blocks,
        v.carried + v.patched + v.edited + v.replaced + v.dropped
    );
    assert!(v.blocks >= 2 && v.dropped == 0, "{v:?}");
    // A tagless file gets its gain; no tag is created for the record, and the row says so.
    assert_eq!(row.notes, ["tags not added: the file has no ID3 tag"]);
    let update = update.as_ref().expect("the row follows the written file");
    let p95 = update.row.short_term_p95.expect("S-P95").0;
    assert!((p95 + 11.0).abs() < 0.05, "{p95} LUFS");
    assert_eq!(update.plan.gain, Some(sc_core::plan::GainPlan::AtTarget));
}

#[test]
fn export_round_trip_testsig() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let hi_res = lib.tone_at("hi-res.wav", 0.1, 96_000);
    let shell = lib.shell(2);
    let ids = analysed(&shell, &[&wav, &hi_res]);
    let before = sha256(&wav);
    let (a, b) = (ids[0], ids[1]);

    let (job, events) = export(&shell, request(&ids, prepare(), None));

    // Per file: in order, ending once.
    assert_eq!(
        steps(&events, a),
        ["started", "processing", "written", "exportDone"]
    );
    assert_eq!(steps(&events, b), ["started", "exportSkipped"]);

    // Every event within its budget, no analysis evidence, and rows step through their stages.
    let mut stages = Vec::new();
    for event in &events {
        let text = serde_json::to_string(event).unwrap();
        assert!(!text.contains("evidence"), "{text}");
        assert!(text.len() <= 2 * MAX_ROW_BYTES, "{} bytes", text.len());
        if let JobEvent::Processing { file_id, row, .. }
        | JobEvent::Written { file_id, row, .. }
        | JobEvent::ExportSkipped { file_id, row, .. }
        | JobEvent::ExportDone { file_id, row, .. } = event
        {
            let bytes = serde_json::to_vec(row).unwrap().len();
            assert!(bytes <= MAX_ROW_BYTES, "{bytes} bytes");
            if *file_id == a {
                stages.push(row.stage);
            } else {
                assert_eq!(row.stage, JobStage::Skipped);
            }
        }
    }
    assert_eq!(
        stages,
        [JobStage::Processing, JobStage::Writing, JobStage::Done]
    );

    written_row_shows_the_export(&events);

    // The skipped file says why.
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::ExportSkipped { row, .. } if row.action == ExportAction::Skip {
            reason: ExportSkip::NotDjSafeRate { sample_rate_hz: 96_000 }
        }
    )));

    // The file changed; the original is in the backups, the artefacts in a new folder.
    assert_ne!(sha256(&wav), before);
    assert!(lib.backups().is_dir());
    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished { job_id, cancelled: false }) if *job_id == job
    ));
    let JobEvent::Artefacts { artefacts, .. } = &events[events.len() - 2] else {
        panic!("{:?}", events[events.len() - 2]);
    };
    assert!(Path::new(&artefacts.folder).starts_with(lib.exports()));
    assert!(Path::new(artefacts.xml.as_ref().unwrap()).is_file());
    let report = std::fs::read_to_string(artefacts.report.as_ref().unwrap()).unwrap();
    assert!(
        report.contains("a.wav") && report.contains("written"),
        "{report}"
    );
    assert!(
        report.contains("hi-res.wav") && report.contains("skipped"),
        "{report}"
    );
    assert_eq!(artefacts.listed, 0, "without a grid the XML lists nothing");
    no_temp_left(&lib.music());

    // A window that reloads gets the same rows back.
    let snapshot = shell.restore();
    let row = snapshot.rows[0].export.as_ref().expect("the export stands");
    assert_eq!(row.stage, JobStage::Done);
    assert_eq!(
        snapshot.rows[1].export.as_ref().expect("skipped").stage,
        JobStage::Skipped
    );
}

#[test]
fn an_export_to_a_folder_leaves_the_source_and_its_row() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);
    let row_before = shell.restore().rows[0].row.clone();
    let out = lib.base().join("out");

    let (_, events) = export(&shell, request(&ids, in_folder(prepare()), Some(&out)));

    assert_eq!(sha256(&wav), before, "the source is untouched");
    let copy = out.join("a.wav");
    assert_ne!(sha256(&copy), before);
    let (row, update) = events
        .iter()
        .find_map(|e| match e {
            JobEvent::ExportDone { row, update, .. } => Some((row, update)),
            _ => None,
        })
        .expect("done");
    assert!(update.is_none(), "the row is about the source");
    assert!(!row.revertible, "a copy leaves nothing to revert");
    assert_eq!(shell.restore().rows[0].row, row_before);
    let JobEvent::Artefacts { artefacts, .. } = &events[events.len() - 2] else {
        panic!("{:?}", events[events.len() - 2]);
    };
    assert_eq!(artefacts.folder, out.display().to_string());
    assert!(out.join("grid-report.csv").is_file());
    assert!(out.join("soundcheck-rekordbox.xml").is_file());

    let refusal = shell.revert(ids[0]).unwrap_err();
    assert_eq!(refusal.kind, IpcErrorKind::NothingToUndo);
    assert_eq!(refusal.file_id, Some(ids[0]));
}

#[test]
fn with_the_xml_off_there_are_no_artefacts() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let settings = ExportSettings {
        xml: false,
        ..prepare()
    };
    let (_, events) = export(&shell, request(&ids, settings, None));
    assert_eq!(
        steps(&events, ids[0]),
        ["started", "processing", "written", "exportDone"]
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        JobEvent::Artefacts { .. } | JobEvent::ArtefactsFailed { .. }
    )));
    assert!(!lib.exports().exists());
}

#[test]
fn artefacts_that_cannot_be_written_are_reported_and_the_files_stand() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    // The exports root is a file, so no folder can be made under it.
    std::fs::write(lib.exports(), b"in the way").unwrap();
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert_eq!(
        steps(&events, ids[0]),
        ["started", "processing", "written", "exportDone"]
    );
    assert!(matches!(
        &events[events.len() - 2],
        JobEvent::ArtefactsFailed { error, .. } if error.kind == IpcErrorKind::Io
    ));
    assert!(matches!(events.last(), Some(JobEvent::Finished { .. })));
}

#[test]
fn export_refused_while_recovery_running() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell_recovering(1);
    assert_eq!(shell.recovery_status(), RecoveryStatus::Running);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);

    let err = shell
        .export(request(&ids, prepare(), None), |_| {})
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::RecoveryRunning, "{err:?}");
    assert_eq!(
        shell.revert(ids[0]).unwrap_err().kind,
        IpcErrorKind::RecoveryRunning
    );
    let xml = WriteXmlRequest {
        file_ids: ids.clone(),
        out: None,
    };
    assert_eq!(
        shell.write_xml(xml).unwrap_err().kind,
        IpcErrorKind::RecoveryRunning
    );
    assert_eq!(
        shell.forget("6ac5b4f4-1-0").unwrap_err().kind,
        IpcErrorKind::RecoveryRunning
    );
    assert_eq!(sha256(&wav), before, "nothing was written");
    assert!(!lib.exports().exists());
    assert!(!lib.backups().exists(), "nothing was journaled");
    assert!(shell.jobs().is_empty(), "no job started");

    // Once recovery ends, the same export runs.
    shell
        .recover_in_background()
        .expect("a recovery thread")
        .join()
        .expect("recovery does not panic");
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert_eq!(
        steps(&events, ids[0]),
        ["started", "processing", "written", "exportDone"]
    );
}

#[test]
fn duplicate_paths_are_refused_before_anything_is_written() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);
    let twice = [ids[0], ids[0]];
    let err = shell
        .export(request(&twice, prepare(), None), |_| {})
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::ListedTwice);
    assert_eq!(err.file_id, Some(ids[0]));

    // The same file under another spelling, added as its own row.
    let spelled = lib.music().join("./a.wav");
    let alias = shell.expand(vec![spelled.display().to_string()]);
    assert_eq!(alias.len(), 1, "a row of its own");
    let both = [ids[0], alias[0].file_id];
    let err = shell
        .export(request(&both, prepare(), None), |_| {})
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::ListedTwice);
    assert_eq!(err.file_id, Some(alias[0].file_id));
    assert_eq!(sha256(&wav), before);

    // Two files one folder would hold under one name.
    let other_dir = lib.base().join("other");
    std::fs::create_dir_all(&other_dir).unwrap();
    let same_name = other_dir.join("a.wav");
    tone_wav(&same_name, 0.2, 44_100);
    let second = analysed(&shell, &[&same_name]);
    let pair = [ids[0], second[0]];
    let other_before = sha256(&same_name);
    let out = lib.base().join("out");
    let (_, events) = export(&shell, request(&pair, in_folder(prepare()), Some(&out)));
    // The first in the list is written, and the later one skipped with why.
    assert!(
        steps(&events, pair[0]).contains(&"exportDone"),
        "{events:?}"
    );
    let skipped = events.iter().find_map(|e| match e {
        JobEvent::ExportSkipped { file_id, row, .. } if *file_id == pair[1] => Some(row.clone()),
        _ => None,
    });
    let skipped = skipped.expect("skipped");
    assert_eq!(
        skipped.action,
        ExportAction::Skip {
            reason: ExportSkip::SameNameInBatch
        }
    );
    assert!(
        skipped.notes[0].contains("earlier in this export"),
        "{:?}",
        skipped.notes
    );
    assert_eq!(sha256(&same_name), other_before, "the source is untouched");
    let preview = shell
        .plan_export(&request(
            &pair,
            in_folder(prepare()),
            Some(&lib.base().join("fresh")),
        ))
        .unwrap();
    assert!(matches!(
        preview.rows[1].row.as_ref().unwrap().action,
        ExportAction::Skip {
            reason: ExportSkip::SameNameInBatch
        }
    ));
    assert!(matches!(
        preview.rows[0].row.as_ref().unwrap().action,
        ExportAction::Write { .. }
    ));
}

#[test]
fn settings_are_checked_at_the_boundary() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);
    let invalid = |req: ExportRequest| {
        let err = shell.export(req, |_| {}).unwrap_err();
        assert_eq!(err.kind, IpcErrorKind::InvalidArgument, "{err:?}");
        err.message
    };

    // The lead floor is the 2 ms fade after the cut; 2 ms itself is allowed (below).
    for lead_ms in [0.0, 1.999, 50.5, f64::NAN] {
        let settings = ExportSettings {
            lead_ms,
            ..prepare()
        };
        let message = invalid(request(&ids, settings, None));
        assert!(message.contains("lead"), "{message}");
    }
    let depth = ExportSettings {
        depth: Some(20),
        ..prepare()
    };
    assert!(invalid(request(&ids, depth, None)).contains("depth"));
    // A folder exactly when the place is a folder, absolute, and a folder.
    let out = lib.base().join("out");
    invalid(request(&ids, in_folder(prepare()), None));
    invalid(request(&ids, prepare(), Some(&out)));
    invalid(request(
        &ids,
        in_folder(prepare()),
        Some(Path::new("relative/out")),
    ));
    invalid(request(&ids, in_folder(prepare()), Some(&wav)));
    // Rows the session holds, at least one.
    invalid(request(&[], prepare(), None));
    let unknown = shell
        .export(request(&[9999], prepare(), None), |_| {})
        .unwrap_err();
    assert_eq!(unknown.file_id, Some(9999));
    assert_eq!(sha256(&wav), before);

    let floor = ExportSettings {
        lead_ms: sc_core::export::LEAD_MS_RANGE.0,
        ..prepare()
    };
    let (_, events) = export(&shell, request(&ids, floor, None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
}

#[test]
fn a_folder_inside_the_backups_is_refused_before_anything_is_written() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.backups().join("copies");
    let err = shell
        .export(request(&ids, in_folder(prepare()), Some(&out)), |_| {})
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::InvalidArgument, "{err:?}");
    assert!(!out.exists(), "no XML or report in the backups");
    // Neither does the XML command write there.
    let refused = shell
        .write_xml(WriteXmlRequest {
            file_ids: ids,
            out: Some(lib.backups().join("mine.xml").display().to_string()),
        })
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::InvalidArgument, "{refused:?}");
}

#[test]
fn a_foreign_artefact_in_the_folder_refuses_the_export() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.base().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("grid-report.csv"), b"the user's own\n").unwrap();
    let err = shell
        .export(request(&ids, in_folder(prepare()), Some(&out)), |_| {})
        .unwrap_err();
    assert_eq!(err.kind, IpcErrorKind::AlreadyExists);
    assert!(!out.join("a.wav").exists());
    // With the XML off, nothing would be written there.
    let no_xml = ExportSettings {
        xml: false,
        ..in_folder(prepare())
    };
    let (_, events) = export(&shell, request(&ids, no_xml, Some(&out)));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    assert_eq!(
        std::fs::read(out.join("grid-report.csv")).unwrap(),
        b"the user's own\n"
    );
}

#[test]
fn cancel_job_stops_an_export_and_leaves_no_temp_file() {
    let lib = Lib::new();
    let files: Vec<PathBuf> = (0..6).map(|i| lib.tone(&format!("{i}.wav"), 0.1)).collect();
    let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    let shell = lib.shell(1);
    let ids = analysed(&shell, &refs);
    let hashes: Vec<Vec<u8>> = files.iter().map(|f| sha256(f)).collect();

    let (tx, rx) = mpsc::channel();
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    // Hold the job at its first event until Cancel has been pressed.
    let mut gate = Some(gate_rx);
    let job = shell
        .export(request(&ids, prepare(), None), move |e| {
            let _ = tx.send(e);
            if let Some(gate) = gate.take() {
                let _ = gate.recv();
            }
        })
        .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(30)).unwrap(),
        JobEvent::Started { .. }
    ));
    assert!(shell.cancel(job), "the job is running");
    gate_tx.send(()).unwrap();
    let events = events_until_finished(&rx);
    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished {
            cancelled: true,
            ..
        })
    ));
    assert!(!shell.cancel(job), "a finished job cannot be cancelled");

    let count = |f: fn(&JobEvent) -> bool| events.iter().filter(|e| f(e)).count();
    let cancelled = count(|e| matches!(e, JobEvent::Cancelled { .. }));
    let done = count(|e| matches!(e, JobEvent::ExportDone { .. }));
    assert_eq!(cancelled + done, 6, "every row ends");
    assert!(cancelled >= 4, "{cancelled} cancelled");
    // No temp file, and every journal entry done or rolled back.
    no_temp_left(&lib.music());
    for entry in txn::journal_entries(&lib.backups()).unwrap() {
        assert!(entry.state.is_final(), "{entry:?}");
    }
    let unchanged = files
        .iter()
        .zip(&hashes)
        .filter(|(f, h)| sha256(f) == **h)
        .count();
    assert_eq!(unchanged, cancelled, "a cancelled file was never replaced");
    // A file that was cancelled has no export standing on its row.
    assert_eq!(
        shell
            .restore()
            .rows
            .iter()
            .filter(|r| r.export.is_some())
            .count(),
        done
    );
}
