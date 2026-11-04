//! Revert, the rekordbox XML command and forgetting a pending change, on synthetic tones, loudness
//! only (no model).

use std::path::{Path, PathBuf};

use sc_core::ipc::{IpcErrorKind, RecoveryStatus, WriteXmlRequest};
use sc_io::cache::Cache;
use sc_io::txn::{self, State};

use super::support::{
    Lib, analysed, date_back, export, loudness_only, mtime, prepare, request, sha256, steps,
    tone_wav,
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
    // The next start finds nothing.
    assert_eq!(lib.shell(1).recovery_status(), clean);
}
