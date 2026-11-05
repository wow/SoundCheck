//! The confirm step's `plan_export` and what a window that reloads reads: synthetic tones, loudness
//! only (no model).

use std::path::Path;

use sc_core::export::{ExportSettings, ExportSkip};
use sc_core::ipc::{
    ConfirmedExport, ConfirmedFile, ExportAction, ExportPreview, ExportRequest, IpcErrorKind,
    JobEvent, JobStage,
};

use super::support::{Lib, analysed, export, held_at, in_folder, prepare, request, sha256, steps};

fn at_processing(e: &JobEvent) -> bool {
    matches!(e, JobEvent::Processing { .. })
}

fn at_written(e: &JobEvent) -> bool {
    matches!(e, JobEvent::Written { .. })
}

fn no_xml(settings: ExportSettings) -> ExportSettings {
    ExportSettings {
        xml: false,
        ..settings
    }
}

#[test]
fn plan_export_says_what_the_job_then_does_and_writes_nothing() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let hi_res = lib.tone_at("hi-res.wav", 0.1, 96_000);
    let shell = lib.shell(2);
    let ids = analysed(&shell, &[&wav, &hi_res]);
    let before = (sha256(&wav), sha256(&hi_res));

    let preview = shell.plan_export(&request(&ids, prepare(), None)).unwrap();

    assert_eq!(
        (sha256(&wav), sha256(&hi_res)),
        before,
        "nothing was written"
    );
    assert!(!lib.exports().exists(), "no folder was made");
    assert!(!lib.backups().exists(), "nothing was journaled");
    assert_eq!(
        preview.backup_folder.as_deref(),
        Some(lib.backups().display().to_string().as_str())
    );
    assert_eq!(
        preview.artefacts_folder.as_deref(),
        Some(lib.exports().display().to_string().as_str())
    );
    let row = |i: usize| preview.rows[i].row.as_ref().expect("planned");
    assert_eq!(preview.rows[0].file_id, ids[0]);
    assert_eq!(row(0).stage, JobStage::Processing);
    assert_eq!(row(1).stage, JobStage::Skipped);
    assert!(
        matches!(
            row(1).action,
            ExportAction::Skip {
                reason: ExportSkip::NotDjSafeRate { .. }
            }
        ),
        "{:?}",
        row(1).action
    );
    assert!(row(1).notes[0].contains("96000 Hz"), "{:?}", row(1).notes);

    // The same action, gain and all, as the job then reports.
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    let planned = events.iter().find_map(|e| match e {
        JobEvent::Processing { row, .. } => Some(row.action.clone()),
        _ => None,
    });
    let skipped = events.iter().find_map(|e| match e {
        JobEvent::ExportSkipped { row, .. } => Some(row.as_ref().clone()),
        _ => None,
    });
    assert_eq!(planned.as_ref(), Some(&row(0).action));
    assert_eq!(
        skipped.as_ref(),
        Some(row(1).as_ref()),
        "the row, wording included"
    );
}

#[test]
fn plan_export_names_the_folders_the_settings_use() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.base().join("copies");

    let to_folder = shell
        .plan_export(&request(&ids, in_folder(prepare()), Some(&out)))
        .unwrap();
    assert_eq!(to_folder.backup_folder, None, "a copy backs nothing up");
    assert_eq!(
        to_folder.artefacts_folder.as_deref(),
        Some(out.display().to_string().as_str())
    );
    assert!(!out.exists(), "not made by asking");

    let xml_off = shell
        .plan_export(&request(&ids, no_xml(prepare()), None))
        .unwrap();
    assert_eq!(xml_off.artefacts_folder, None);
}

#[test]
fn plan_export_refuses_what_the_export_refuses() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);

    // A folder inside the backups.
    let refused = shell
        .plan_export(&request(
            &ids,
            in_folder(prepare()),
            Some(&lib.backups().join("x")),
        ))
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::InvalidArgument, "{refused:?}");
    // A file named twice.
    let refused = shell
        .plan_export(&request(&[ids[0], ids[0]], prepare(), None))
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::ListedTwice, "{refused:?}");

    // During recovery, on the same refusal as the commands around it.
    let recovering = lib.shell_recovering(1);
    let ids = analysed(&recovering, &[&wav]);
    let refused = recovering
        .plan_export(&request(&ids, prepare(), None))
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::RecoveryRunning, "{refused:?}");
}

#[test]
fn plan_export_is_refused_while_the_file_is_being_exported() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);

    let held = held_at(&shell, request(&ids, prepare(), None), at_processing);
    let refused = shell
        .plan_export(&request(&ids, prepare(), None))
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::Busy, "{refused:?}");
    held.finish();

    // The hold it takes is its own: nothing stays busy after it answers.
    shell.plan_export(&request(&ids, prepare(), None)).unwrap();
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
}

#[test]
fn a_row_not_analysed_yet_says_so_on_its_row() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let id = shell.expand(vec![wav.display().to_string()])[0].file_id;

    let preview = shell.plan_export(&request(&[id], prepare(), None)).unwrap();

    assert_eq!(preview.rows[0].row, None);
    let error = preview.rows[0].error.as_ref().expect("an error");
    assert_eq!(error.file_id, Some(id));
    assert!(error.message.contains("not analysed"), "{error:?}");
}

#[test]
fn a_reload_during_an_export_is_told_the_job_runs_and_then_finds_its_artefacts() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.base().join("copies");
    let req = request(&ids, in_folder(prepare()), Some(&out));

    // Past the point where a cancel can stop the file: it ends as written, and the batch has
    // artefacts the old page's channel would have reported.
    let held = held_at(&shell, req, at_written);
    let during = shell.restore();
    assert_eq!(during.running_jobs.len(), 1, "named while it winds down");
    assert_eq!(
        during.rows[0].export.as_ref().map(|r| r.stage),
        Some(JobStage::Writing),
        "the row in flight"
    );
    assert_eq!(during.artefacts, None, "not written yet");

    let events = held.finish();

    // The page that reloaded reads again: no job left, and what the old page was told is here too.
    let after = shell.restore();
    assert_eq!(after.running_jobs, Vec::<u32>::new());
    let told = events.iter().find_map(|e| match e {
        JobEvent::Artefacts { artefacts, .. } => Some(artefacts.clone()),
        _ => None,
    });
    assert!(told.is_some(), "{events:?}");
    assert_eq!(after.artefacts, told);
    assert_eq!(after.artefacts_failed, None);
}

#[test]
fn the_next_batch_or_clearing_the_list_drops_the_artefacts() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.base().join("copies");

    export(&shell, request(&ids, in_folder(prepare()), Some(&out)));
    assert!(shell.restore().artefacts.is_some());
    // The next batch has none, with its XML off.
    export(
        &shell,
        request(
            &ids,
            no_xml(in_folder(prepare())),
            Some(&lib.base().join("more")),
        ),
    );
    assert_eq!(shell.restore().artefacts, None);

    export(
        &shell,
        request(&ids, in_folder(prepare()), Some(&lib.base().join("again"))),
    );
    assert!(shell.restore().artefacts.is_some());
    shell.clear();
    assert_eq!(shell.restore().artefacts, None);
}

/// The request sent back after `preview`: what the user confirmed.
fn confirmed_of(mut req: ExportRequest, preview: &ExportPreview) -> ExportRequest {
    req.confirmed = Some(ConfirmedExport {
        revision: preview.revision,
        files: preview
            .rows
            .iter()
            .map(|r| ConfirmedFile {
                file_id: r.file_id,
                identity: r.identity.clone().expect("planned"),
            })
            .collect(),
    });
    req
}

/// Another tone of a different level, of another length so the file is told apart at once.
fn rewrite(wav: &Path) {
    super::support::tone_wav(wav, 0.5, 44_100);
    let mut bytes = std::fs::read(wav).unwrap();
    bytes.extend_from_slice(&[0; 2]);
    std::fs::write(wav, &bytes).unwrap();
}

#[test]
fn a_file_changed_after_confirm_fails_its_row_and_is_not_written() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let other = lib.tone("b.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav, &other]);
    let req = request(&ids, prepare(), None);
    let preview = shell.plan_export(&req).unwrap();
    let req = confirmed_of(req, &preview);

    rewrite(&wav);
    let rewritten = sha256(&wav);
    let (_, events) = export(&shell, req);

    assert_eq!(sha256(&wav), rewritten, "left as it is");
    let refused = events.iter().find_map(|e| match e {
        JobEvent::Failed { file_id, error, .. } if *file_id == ids[0] => Some(error),
        _ => None,
    });
    let refused = refused.expect("failed");
    assert_eq!(refused.kind, IpcErrorKind::FileChanged, "{refused:?}");
    assert!(
        refused.message.contains("since you confirmed"),
        "{refused:?}"
    );
    // The other file of the batch goes on.
    assert!(steps(&events, ids[1]).contains(&"exportDone"));
}

#[test]
fn a_file_unchanged_since_confirm_is_written() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let req = request(&ids, prepare(), None);
    let preview = shell.plan_export(&req).unwrap();
    let (_, events) = export(&shell, confirmed_of(req, &preview));
    assert!(steps(&events, ids[0]).contains(&"exportDone"), "{events:?}");
}

#[test]
fn the_preview_notices_a_file_that_changed_since_its_analysis() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    rewrite(&wav);
    let preview = shell.plan_export(&request(&ids, prepare(), None)).unwrap();
    assert_eq!(preview.rows[0].row, None);
    let error = preview.rows[0].error.as_ref().expect("an error");
    assert_eq!(error.kind, IpcErrorKind::FileChanged);
    assert!(error.message.contains("since it was analysed"), "{error:?}");
}

#[test]
fn the_job_refuses_settings_that_changed_after_the_plan() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);
    let req = request(&ids, prepare(), None);
    let preview = shell.plan_export(&req).unwrap();
    assert!((preview.decide.target.0 + 11.0).abs() < 1e-9);

    let mut decide = preview.decide;
    decide.target = sc_core::Lufs(-8.0);
    shell.set_decide_settings(decide).unwrap();

    let refused = shell
        .export(confirmed_of(req, &preview), |_| {})
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::SettingsChanged, "{refused:?}");
    assert_eq!(sha256(&wav), before, "nothing was written");
}

#[test]
fn the_preview_says_which_copies_it_replaces_and_which_files_it_leaves() {
    let lib = Lib::new();
    let (a, b, c) = (
        lib.tone("a.wav", 0.1),
        lib.tone("b.wav", 0.1),
        lib.tone("c.wav", 0.1),
    );
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&a, &b, &c]);
    let out = lib.base().join("copies");
    let (_, events) = export(&shell, request(&ids[..1], in_folder(prepare()), Some(&out)));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    std::fs::write(out.join("b.wav"), b"someone else's").unwrap();

    let preview = shell
        .plan_export(&request(&ids, in_folder(prepare()), Some(&out)))
        .unwrap();

    assert!(preview.rows[0].replaces_copy, "an earlier copy");
    assert!(!preview.rows[1].replaces_copy && !preview.rows[2].replaces_copy);
    let action = |i: usize| preview.rows[i].row.as_ref().unwrap().action.clone();
    assert!(matches!(action(0), ExportAction::Write { .. }));
    assert_eq!(
        action(1),
        ExportAction::Skip {
            reason: ExportSkip::OtherFileInFolder
        }
    );
    assert!(matches!(action(2), ExportAction::Write { .. }));
    assert_eq!(std::fs::read(out.join("b.wav")).unwrap(), b"someone else's");
}

#[test]
fn a_row_says_what_it_was_exported_with() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.base().join("copies");
    export(&shell, request(&ids, in_folder(prepare()), Some(&out)));
    let snapshot = shell.restore();
    let row = snapshot.rows[0].export.as_ref().expect("exported");
    let with = row.exported_with.as_ref().expect("recorded");
    assert!((with.decide.target.0 + 11.0).abs() < 1e-9);
    assert_eq!(with.settings.place, sc_core::export::Place::Folder);
    assert_eq!(
        with.out_dir.as_deref(),
        Some(out.display().to_string().as_str())
    );
}

#[test]
fn the_snapshot_holds_what_became_of_the_latest_batch() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let hi_res = lib.tone_at("hi-res.wav", 0.1, 96_000);
    let shell = lib.shell(2);
    let ids = analysed(&shell, &[&wav, &hi_res]);
    assert_eq!(shell.restore().last_export, None, "only an export counts");

    let out = lib.base().join("copies");
    let (job, _) = export(&shell, request(&ids, in_folder(prepare()), Some(&out)));

    let summary = shell.restore().last_export.expect("kept");
    assert_eq!(summary.job_id, job);
    assert_eq!(
        (summary.written, summary.skipped, summary.failed),
        (1, 1, 0)
    );
    assert!(!summary.cancelled);
    shell.clear();
    assert_eq!(shell.restore().last_export, None);
}

#[test]
fn a_cancelled_file_is_not_counted_as_failed() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let held = held_at(&shell, request(&ids, prepare(), None), at_processing);
    let _ = shell.restore();
    held.finish();
    let summary = shell.restore().last_export.expect("kept");
    assert_eq!((summary.written, summary.failed), (0, 0));
    assert!(summary.cancelled);
}
