//! Revert, the rekordbox XML command and forgetting a pending change, on synthetic tones, loudness
//! only (no model).

use std::path::{Path, PathBuf};

use sc_core::Seconds;
use sc_core::export::{Cut, ExportPlan, GridCheck, GridCheckSkip, XmlGrid, XmlTrackInfo};
use sc_core::ipc::{IpcErrorKind, JobEvent, RecoveryStatus, WriteXmlRequest};
use sc_engine::{EditState, EngineEvent, ProcessDone};
use sc_io::cache::Cache;
use sc_io::render::RenderReport;
use sc_io::txn::{self, State, TxnKind, TxnReport};

use super::support::{
    Lib, analysed, date_back, export, in_folder, loudness_only, mtime, prepare, request, sha256,
    steps, tone_wav,
};

#[test]
fn revert_restores_hash() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let dated = date_back(&wav);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let id = ids[0];
    let before = sha256(&wav);
    let row_before = shell.restore().rows[0].row.clone().expect("analysed");

    // Prepare does not keep the modification time, so both the bytes and the time change.
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, id).contains(&"exportDone"));
    assert_ne!(sha256(&wav), before);
    assert_ne!(mtime(&wav), dated);
    let exported = shell.restore();
    assert_ne!(
        exported.rows[0].row.as_ref().unwrap().short_term_p95,
        row_before.short_term_p95,
        "the row follows the written file"
    );
    assert!(exported.rows[0].export.is_some());

    let reverted = shell.revert(id).unwrap();

    assert_eq!(sha256(&wav), before, "the original's bytes");
    assert_eq!(mtime(&wav), dated, "the original's modification time");
    assert_eq!(reverted.earlier_changes, 0);
    assert_eq!(reverted.notes, Vec::<String>::new());
    assert!(Path::new(&reverted.backup).is_file(), "the backup is kept");
    // The row shows the original's measurements again, and they are what the cache now serves.
    let update = reverted.update.expect("analysed again");
    assert_eq!(update.file_id, id);
    assert_eq!(update.row.short_term_p95, row_before.short_term_p95);
    assert_eq!(update.row.integrated, row_before.integrated);
    assert_eq!(update.row.true_peak, row_before.true_peak);
    assert!(!update.row.cached);
    let (nfc, key) = Cache::key_for(&wav, &loudness_only()).unwrap();
    let cached = lib.cache().get(&nfc, &key).expect("the original is cached");
    assert_eq!(cached.loudness.short_term_p95, row_before.short_term_p95);
    let snapshot = shell.restore();
    assert_eq!(snapshot.rows[0].export, None, "no export stands");
    assert_eq!(
        snapshot.rows[0].row.as_ref().unwrap().short_term_p95,
        row_before.short_term_p95
    );

    // Nothing left to revert: a typed refusal, and nothing moves.
    let refusal = shell.revert(id).unwrap_err();
    assert_eq!(refusal.kind, IpcErrorKind::NothingToUndo);
    assert_eq!(refusal.file_id, Some(id));
    assert_eq!(sha256(&wav), before);
    assert_eq!(
        shell.revert(9999).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
}

#[test]
fn a_revert_whose_analysis_fails_leaves_the_original_and_no_cached_output() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));

    let reverted = shell
        .revert_analysing(ids[0], |_| {
            Err(sc_core::Error::Internal("the analysis broke".into()))
        })
        .unwrap();

    assert_eq!(sha256(&wav), before, "the file is reverted all the same");
    assert!(reverted.update.is_none());
    assert_eq!(reverted.notes.len(), 1);
    assert!(
        reverted.notes[0].contains("the analysis broke"),
        "{:?}",
        reverted.notes
    );
    // What the export left in the cache went with the undo: nothing about the output is served.
    assert!(!lib.cache().remove(&wav).unwrap(), "no entry for the file");
    // The row no longer claims measurements of a file that is gone, and no export stands.
    let snapshot = shell.restore();
    assert_eq!(snapshot.rows[0].row, None);
    assert_eq!(snapshot.rows[0].plan, None);
    assert_eq!(snapshot.rows[0].export, None);
}

#[test]
fn revert_refuses_a_file_edited_since_it_was_written() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    tone_wav(&wav, 0.2, 44_100);
    let edited = sha256(&wav);
    let refusal = shell.revert(ids[0]).unwrap_err();
    assert_eq!(refusal.kind, IpcErrorKind::FileChanged);
    assert_eq!(sha256(&wav), edited);
}

#[test]
fn the_xml_command_writes_to_the_default_folder_or_the_file_named() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let before = sha256(&wav);

    // The default: a new folder under the exports folder, with the grid report.
    let written = shell
        .write_xml(WriteXmlRequest {
            file_ids: ids.clone(),
            out: None,
        })
        .unwrap();
    let folder = PathBuf::from(&written.artefacts.folder);
    assert!(folder.starts_with(lib.exports()), "{folder:?}");
    assert!(folder.join("soundcheck-rekordbox.xml").is_file());
    assert!(folder.join("grid-report.csv").is_file());
    assert_eq!(written.artefacts.listed, 0);
    // Loudness only has no grid, so the file is not listed, and that says why.
    assert_eq!(written.not_listed.len(), 1);
    assert_eq!(written.not_listed[0].file_id, ids[0]);
    assert!(
        written.not_listed[0].why.contains("no grid"),
        "{:?}",
        written.not_listed
    );
    assert_eq!(sha256(&wav), before, "the XML command writes no audio");

    // A file named: the XML alone, replacing only what SoundCheck wrote.
    let out = lib.base().join("mine.xml");
    let named = |out: &Path| WriteXmlRequest {
        file_ids: ids.clone(),
        out: Some(out.display().to_string()),
    };
    let written = shell.write_xml(named(&out)).unwrap();
    assert_eq!(written.artefacts.xml, Some(out.display().to_string()));
    assert_eq!(written.artefacts.report, None);
    assert!(out.is_file());
    shell
        .write_xml(named(&out))
        .expect("its own file is replaced");

    // The guards of the command line's `xml`.
    let refused = |out: &Path| shell.write_xml(named(out)).unwrap_err().kind;
    assert_eq!(refused(&wav), IpcErrorKind::InvalidArgument);
    assert_eq!(
        refused(Path::new("relative.xml")),
        IpcErrorKind::InvalidArgument
    );
    let foreign = lib.base().join("other-app.xml");
    std::fs::write(&foreign, b"<DJ_PLAYLISTS/>").unwrap();
    assert_eq!(refused(&foreign), IpcErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&foreign).unwrap(), b"<DJ_PLAYLISTS/>");
    let twice = shell
        .write_xml(WriteXmlRequest {
            file_ids: vec![ids[0], ids[0]],
            out: None,
        })
        .unwrap_err();
    assert_eq!(twice.kind, IpcErrorKind::ListedTwice);
}

/// A change a crash left on a volume that is not mounted, so recovery leaves it pending.
fn pending_change(lib: &Lib, txn: &str) -> PathBuf {
    let gone = lib.base().join("Volumes/Gone/Track.wav");
    let temp = gone.with_file_name(format!(".Track.wav.soundcheck-tmp-{txn}"));
    let line = serde_json::json!({
        "txn": txn, "state": "planned", "at": "2026-10-07T10:00:00Z",
        "kind": "in_place", "path": gone, "source": gone, "temp": temp,
    });
    std::fs::create_dir_all(lib.backups()).unwrap();
    std::fs::write(lib.backups().join("journal.jsonl"), format!("{line}\n")).unwrap();
    gone
}

#[test]
fn a_pending_change_is_listed_and_can_be_forgotten() {
    let lib = Lib::new();
    let txn = "6ac5b4f4-9-0";
    let gone = pending_change(&lib, txn);
    let shell = lib.shell(1);

    // What the recovery banner needs: path and reason.
    let RecoveryStatus::Finished { pending, .. } = shell.recovery_status() else {
        panic!("{:?}", shell.recovery_status());
    };
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].txn, txn);
    assert_eq!(pending[0].path, gone.display().to_string());
    assert!(
        pending[0].reason.contains("not reachable"),
        "{:?}",
        pending[0]
    );

    // Pending changes do not block the rest of the library.
    let wav = lib.tone("a.wav", 0.1);
    let ids = analysed(&shell, &[&wav]);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));

    // Unfinished in the journal, but not what recovery reported: it may be running.
    let unlisted = "6ac5b4f4-9-2";
    let line = serde_json::json!({
        "txn": unlisted, "state": "planned", "at": "2026-10-07T10:00:00Z",
        "kind": "in_place", "path": gone, "source": gone,
        "temp": gone.with_file_name(format!(".Track.wav.soundcheck-tmp-{unlisted}")),
    });
    let journal = lib.backups().join("journal.jsonl");
    let mut text = std::fs::read_to_string(&journal).unwrap();
    text.push_str(&format!("{line}\n"));
    std::fs::write(&journal, text).unwrap();
    assert_eq!(
        shell.forget(unlisted).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
    let entries = txn::journal_entries(&lib.backups()).unwrap();
    let entry = entries
        .iter()
        .find(|e| e.txn == unlisted)
        .expect("journaled");
    assert!(!entry.state.is_final(), "left as it was");

    // An id recovery did not leave pending is refused, the finished export's included.
    let finished = txn::journal_entries(&lib.backups())
        .unwrap()
        .iter()
        .find(|e| e.state.is_final())
        .map(|e| e.txn.clone())
        .expect("the export is journaled");
    for id in [finished.as_str(), "6ac5b4f4-9-1", "not an id"] {
        assert_eq!(
            shell.forget(id).unwrap_err().kind,
            IpcErrorKind::InvalidArgument,
            "{id}"
        );
    }

    let forgotten = shell.forget(txn).unwrap();
    assert_eq!(forgotten.path, gone.display().to_string());
    let clean = RecoveryStatus::Finished {
        recovered: vec![],
        pending: vec![],
    };
    assert_eq!(forgotten.status, clean);
    assert_eq!(shell.recovery_status(), clean);
    let entries = txn::journal_entries(&lib.backups()).unwrap();
    let entry = entries.iter().find(|e| e.txn == txn).expect("journaled");
    assert_eq!(entry.state, State::Forgotten);
    assert_eq!(
        shell.forget(txn).unwrap_err().kind,
        IpcErrorKind::InvalidArgument,
        "only once"
    );
}

#[test]
fn two_forgets_at_once_lose_neither() {
    let lib = Lib::new();
    let ids: Vec<String> = (0..8).map(|n| format!("6ac5b4f4-9-{n}")).collect();
    let gone = lib.base().join("Volumes/Gone/Track.wav");
    let lines: String = ids
        .iter()
        .map(|txn| {
            let line = serde_json::json!({
                "txn": txn, "state": "planned", "at": "2026-10-07T10:00:00Z",
                "kind": "in_place", "path": gone, "source": gone,
                "temp": gone.with_file_name(format!(".Track.wav.soundcheck-tmp-{txn}")),
            });
            format!("{line}\n")
        })
        .collect();
    std::fs::create_dir_all(lib.backups()).unwrap();
    std::fs::write(lib.backups().join("journal.jsonl"), lines).unwrap();
    let shell = lib.shell(1);
    std::thread::scope(|scope| {
        for id in &ids {
            let shell = shell.clone();
            scope.spawn(move || shell.forget(id).unwrap());
        }
    });
    assert_eq!(
        shell.recovery_status(),
        RecoveryStatus::Finished {
            recovered: vec![],
            pending: vec![]
        }
    );
}

#[test]
fn a_revert_that_leaves_an_earlier_change_keeps_the_row_revertible() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let original = sha256(&wav);
    assert!(!shell.restore().rows[0].revertible, "nothing to put back");
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    assert!(shell.restore().rows[0].revertible);
    let first = sha256(&wav);
    // A second change: another target, so the file changes again.
    shell
        .set_decide_settings(sc_core::plan::DecideSettings {
            target: sc_core::Lufs(-14.0),
            ..sc_core::plan::DecideSettings::dj()
        })
        .unwrap();
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    assert_ne!(sha256(&wav), first);

    let reverted = shell.revert(ids[0]).unwrap();
    assert_eq!(sha256(&wav), first);
    assert_eq!(reverted.earlier_changes, 1);
    let snapshot = shell.restore();
    assert_eq!(snapshot.rows[0].export, None, "that export is gone");
    assert!(snapshot.rows[0].revertible, "a reload still offers Revert");

    // The next export of it that is skipped or copied says the earlier change stands.
    let out = lib.base().join("out");
    let (_, events) = export(&shell, request(&ids, in_folder(prepare()), Some(&out)));
    let row = events
        .iter()
        .find_map(|e| match e {
            JobEvent::ExportDone { row, in_place, .. } => {
                assert!(!in_place);
                Some(row)
            }
            _ => None,
        })
        .expect("done");
    assert!(row.revertible, "the earlier change still stands");

    let reverted = shell.revert(ids[0]).unwrap();
    assert_eq!(sha256(&wav), original);
    assert_eq!(reverted.earlier_changes, 0);
    assert!(!shell.restore().rows[0].revertible);
}

#[test]
fn a_skip_after_an_export_in_place_keeps_it_revertible() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    let (_, events) = export(&shell, request(&ids, prepare(), None));
    assert!(steps(&events, ids[0]).contains(&"exportDone"));
    // Grid only on a file without a grid is skipped.
    let grid_only = sc_core::export::ExportSettings {
        grid_only: true,
        ..prepare()
    };
    let (_, events) = export(&shell, request(&ids, grid_only, None));
    assert_eq!(steps(&events, ids[0]), ["started", "exportSkipped"]);
    let row = events
        .iter()
        .find_map(|e| match e {
            JobEvent::ExportSkipped { row, .. } => Some(row),
            _ => None,
        })
        .expect("skipped");
    assert!(row.revertible, "the export before it still stands");
    assert!(shell.restore().rows[0].export.as_ref().unwrap().revertible);
    assert!(shell.revert(ids[0]).is_ok());
}

/// The events of a file written in place whose own analysis did not finish. Nothing can make that
/// analysis fail on demand, so these are what the batch sends then.
fn events_of_an_unfinished_analysis(wav: &Path) -> [EngineEvent; 3] {
    let plan = ExportPlan {
        gain_db: 3.0,
        trim_frames: 0,
        trim_snapped_from_frames: None,
        expect_frames: 1000,
        bits: None,
        tags: vec![],
        bext: None,
        cut: Cut::NoGrid,
        notices: vec![],
        grid_withheld: false,
    };
    let report = TxnReport {
        txn: "6ac5b4f4-1-0".to_owned(),
        kind: TxnKind::InPlace,
        source: wav.to_path_buf(),
        output: wav.to_path_buf(),
        backup: None,
        sidecar: None,
        render: RenderReport {
            tags_added: false,
            tags_not_added: None,
            tag_edit: None,
            vorbis_edit: None,
            frames_in: 1000,
            frames_out: 1000,
            trim_frames: 0,
            trim_requested_frames: 0,
            sample_rate_hz: 44_100,
            channels: 2,
            bits_out: 16,
            exact: false,
            dithered: false,
            samples_saturated: 0,
            pcm_hash: [0; 32],
            stale_loudness_tags: vec![],
            blocks: vec![],
            leading_bytes: 0,
            trailing_bytes: 0,
            output_bytes: 0,
        },
        original_blake3: [0; 32],
        output_blake3: [0; 32],
        output_bytes: 0,
        notes: vec![],
        timings: vec![],
    };
    let done = ProcessDone {
        output: wav.to_path_buf(),
        duration: Seconds(5.0),
        xml: XmlTrackInfo {
            grid: XmlGrid::Absent,
            title: None,
            artist: None,
        },
        analysis: None,
        edit: EditState::default(),
        edit_carried: false,
        grid_check: GridCheck::NotChecked {
            reason: GridCheckSkip::NotAnalysed,
        },
        notes: vec!["the written file could not be analysed: it broke".to_owned()],
    };
    [
        EngineEvent::Processing {
            file_id: 1,
            plan: Box::new(plan),
        },
        EngineEvent::Written {
            file_id: 1,
            report: Box::new(report),
        },
        EngineEvent::Done {
            file_id: 1,
            done: Box::new(done),
        },
    ]
}

#[test]
fn an_export_in_place_whose_analysis_did_not_finish_clears_the_rows_numbers() {
    let lib = Lib::new();
    let wav = lib.tone("a.wav", 0.1);
    let shell = lib.shell(1);
    let ids = analysed(&shell, &[&wav]);
    assert_eq!(ids, [1]);
    assert!(shell.restore().rows[0].row.is_some());

    let mut last = None;
    for event in events_of_an_unfinished_analysis(&wav) {
        last = shell.session().on_engine_event(1, event);
    }

    let Some(JobEvent::ExportDone {
        row,
        in_place,
        update,
        ..
    }) = last
    else {
        panic!("{last:?}");
    };
    assert!(in_place, "the file was replaced");
    assert_eq!(update, None, "and there is nothing to show for it");
    assert!(
        row.notes[0].contains("could not be analysed"),
        "{:?}",
        row.notes
    );
    let snapshot = shell.restore();
    assert_eq!(
        snapshot.rows[0].row, None,
        "what was measured no longer holds"
    );
    assert_eq!(snapshot.rows[0].plan, None);
    assert!(snapshot.rows[0].revertible);
}
