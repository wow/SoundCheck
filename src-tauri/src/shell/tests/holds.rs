//! Two jobs or commands on one file: refused with `busy`, each row ending as what its one job did,
//! and the hold released on every way out. Synthetic tones, loudness only (no model).

use std::time::Duration;

use sc_core::ipc::{IpcErrorKind, JobEvent, WriteXmlRequest};
use sc_io::txn;

use super::super::HoldKind;
use super::support::{
    Lib, analysed, analysis_held_at, events_until_finished, export, held_at, in_folder, prepare,
    request, sha256, steps,
};

fn at_started(e: &JobEvent) -> bool {
    matches!(e, JobEvent::Started { .. })
}

fn at_written(e: &JobEvent) -> bool {
    matches!(e, JobEvent::Written { .. })
}

#[test]
fn export_pressed_twice_refuses_the_second_and_the_first_row_stands() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);

    let held = held_at(&shell, request(&ids, prepare(), None), at_started);
    let refused = shell
        .export(request(&ids, prepare(), None), |_| {})
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::Busy);
    assert_eq!(refused.file_id, Some(ids[0]));
    let events = held.finish();

    assert_eq!(
        steps(&events, ids[0]),
        ["started", "processing", "written", "exportDone"]
    );
    // The row shows the written file: at the target, with its export and Revert.
    let snapshot = shell.restore();
    let row = snapshot.rows[0].row.as_ref().expect("analysed");
    let p95 = row.short_term_p95.expect("S-P95").0;
    assert!((p95 + 11.0).abs() < 0.05, "{p95} LUFS");
    assert!(snapshot.rows[0].export.is_some());
    assert!(snapshot.rows[0].revertible);
    assert_eq!(
        txn::journal_entries(&lib.backups()).unwrap().len(),
        1,
        "one change"
    );
}

#[test]
fn revert_during_an_uncached_export_analysis_is_refused() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let original = sha256(&wav);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    let exported = sha256(&wav);

    // No cached analysis: the next export analyses the exported bytes first.
    assert!(lib.cache().remove(&wav).unwrap());
    let held = held_at(&shell, request(&ids, prepare(), None), at_started);
    let refused = shell.revert(ids[0]).unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::Busy);
    assert_eq!(refused.file_id, Some(ids[0]));
    let events = held.finish();

    assert!(
        !events.iter().any(|e| matches!(e, JobEvent::Failed { .. })),
        "{events:?}"
    );
    // The refusal changed nothing, and each change can be put back, down to the original.
    assert_eq!(sha256(&wav), exported, "written at the target, 0 dB");
    while shell.revert(ids[0]).is_ok() {}
    assert_eq!(sha256(&wav), original);
}

#[test]
fn revert_after_written_is_refused_so_the_report_and_row_stay_true() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let original = sha256(&wav);

    let held = held_at(&shell, request(&ids, prepare(), None), at_written);
    let written = sha256(&wav);
    assert_ne!(written, original, "in place already");
    let refused = shell.revert(ids[0]).unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::Busy);
    assert_eq!(sha256(&wav), written);
    let events = held.finish();

    // What the batch reported is what the file holds.
    assert_eq!(sha256(&wav), written);
    let Some(JobEvent::ExportDone { update, .. }) = events
        .iter()
        .find(|e| matches!(e, JobEvent::ExportDone { .. }))
    else {
        panic!("{events:?}");
    };
    assert!(update.is_some());
    let JobEvent::Artefacts { artefacts, .. } = &events[events.len() - 2] else {
        panic!("{:?}", events[events.len() - 2]);
    };
    let report = std::fs::read_to_string(artefacts.report.as_ref().unwrap()).unwrap();
    assert!(report.contains("written"), "{report}");

    shell.revert(ids[0]).unwrap();
    assert_eq!(sha256(&wav), original);
}

#[test]
fn a_file_held_by_an_analysis_cannot_be_changed_and_analyses_share_it() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let held = analysis_held_at(&shell, &ids, at_started);
    for kind in [
        shell
            .export(request(&ids, prepare(), None), |_| {})
            .unwrap_err()
            .kind,
        shell.revert(ids[0]).unwrap_err().kind,
    ] {
        assert_eq!(kind, IpcErrorKind::Busy);
    }
    // A second analysis of it runs (the UI replaces a cancelled one with it).
    let second = analysis_held_at(&shell, &ids, at_started);
    second.finish();
    held.finish();
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
}

#[test]
fn a_file_being_exported_cannot_be_analysed_listed_or_opened() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let held = held_at(&shell, request(&ids, prepare(), None), at_started);
    let req = sc_core::ipc::AnalyzeRequest {
        file_ids: ids.clone(),
        analysis: super::support::loudness_only(),
    };
    assert_eq!(
        shell.start(req, |_| {}).unwrap_err().kind,
        IpcErrorKind::Busy
    );
    let xml = WriteXmlRequest {
        file_ids: ids.clone(),
        out: None,
    };
    assert_eq!(shell.write_xml(xml).unwrap_err().kind, IpcErrorKind::Busy);
    // A view of the old version would save an edit over the new row's grid.
    assert_eq!(
        shell.open_track(ids[0], |_| {}).unwrap_err().kind,
        IpcErrorKind::Busy
    );
    held.finish();
}

#[test]
fn a_revert_holds_the_file_throughout_so_a_second_press_undoes_nothing() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let original = sha256(&wav);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    let inner = shell.clone();
    let id = ids[0];
    let reverted = shell
        .revert_analysing(id, move |_| {
            // The file is back, and the first press has not ended.
            let busy = |kind: IpcErrorKind| assert_eq!(kind, IpcErrorKind::Busy);
            busy(inner.revert(id).unwrap_err().kind);
            busy(
                inner
                    .export(request(&[id], prepare(), None), |_| {})
                    .unwrap_err()
                    .kind,
            );
            busy(inner.open_track(id, |_| {}).unwrap_err().kind);
            Err(sc_core::Error::Internal("not analysed in this test".into()))
        })
        .unwrap();
    assert_eq!(reverted.earlier_changes, 0);
    assert_eq!(sha256(&wav), original);
}

#[test]
fn a_copy_to_a_folder_is_refused_on_a_file_being_exported_in_place() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let out = lib.base().join("out");
    let held = held_at(&shell, request(&ids, prepare(), None), at_started);
    let refused = shell
        .export(request(&ids, in_folder(prepare()), Some(&out)), |_| {})
        .unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::Busy);
    held.finish();
    assert!(!out.exists());
}

#[test]
fn the_hold_is_released_after_a_failed_or_cancelled_job_before_finished_goes_out() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let broken = lib.music().join("broken.wav");
    std::fs::write(&broken, b"RIFF\x04\x00\x00\x00WAVEnot a real wave file").unwrap();
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let broken_id = shell.expand(vec![broken.display().to_string()])[0].file_id;

    // Failed: an export again the moment `finished` arrives.
    let (_, events) = export(&shell, request(&[broken_id], prepare(), None));
    assert_eq!(steps(&events, broken_id), ["started", "failed"]);
    let (_, events) = export(&shell, request(&[broken_id], prepare(), None));
    assert_eq!(steps(&events, broken_id), ["started", "failed"]);

    // Cancelled: the same.
    let (tx, rx) = std::sync::mpsc::channel();
    let job = shell
        .export(request(&ids, prepare(), None), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
    assert!(shell.cancel(job), "it is running");
    let events = events_until_finished(&rx);
    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished { job_id, .. }) if *job_id == job
    ));
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));

    // A refused revert: a second one gets its own answer, not `busy`.
    let refused = shell.revert(broken_id).unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::NothingToUndo);
    let refused = shell.revert(broken_id).unwrap_err();
    assert_eq!(refused.kind, IpcErrorKind::NothingToUndo);
}

#[test]
fn a_job_that_panics_releases_its_files() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let hold = shell
        .inner
        .holds
        .acquire(&[(ids[0], wav.as_path())], HoldKind::ExportInPlace)
        .unwrap();
    assert_eq!(
        shell.revert(ids[0]).unwrap_err().kind,
        IpcErrorKind::Busy,
        "held"
    );
    shell.spawn_job(999, hold, |_| {}, |_, _, _| panic!("the job broke"));
    let released = (0..500).any(|_| {
        std::thread::sleep(Duration::from_millis(10));
        shell.inner.holds.refuse_changing(ids[0], &wav).is_ok()
    });
    assert!(released, "still held after the job's thread ended");
}
