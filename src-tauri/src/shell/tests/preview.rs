//! The confirm step's `plan_export` and what a window that reloads reads: synthetic tones, loudness
//! only (no model).

use sc_core::export::{ExportSettings, ExportSkip};
use sc_core::ipc::{ExportAction, IpcErrorKind, JobEvent, JobStage};

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
