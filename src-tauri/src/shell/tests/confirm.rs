//! What the confirm step says is what the job does: only a row that plans a write takes a file
//! name, the job keeps to what the confirm decided, and an export in place refused later is
//! refused in the plan, which writes nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sc_core::export::{ExportSettings, ExportSkip};
use sc_core::ipc::{
    ConfirmedExport, ConfirmedFile, ExportAction, ExportPreview, ExportPreviewRow, ExportRequest,
    IpcErrorKind, JobEvent,
};

use super::support::{Lib, analysed, export, in_folder, prepare, request, sha256, steps, tone_wav};

fn no_xml(settings: ExportSettings) -> ExportSettings {
    ExportSettings {
        xml: false,
        ..settings
    }
}

fn to_folder() -> ExportSettings {
    no_xml(in_folder(prepare()))
}

fn in_place() -> ExportSettings {
    no_xml(prepare())
}

fn name_taken(row: &ExportPreviewRow) -> bool {
    row.row.as_ref().is_some_and(|r| {
        r.action
            == ExportAction::Skip {
                reason: ExportSkip::SameNameInBatch,
            }
    })
}

fn writes(row: &ExportPreviewRow) -> bool {
    matches!(
        row.row.as_ref().map(|r| &r.action),
        Some(ExportAction::Write { .. })
    )
}

/// What the app sends after `preview`: the rows it could plan, each with what the plan decided.
fn confirmed(mut req: ExportRequest, preview: &ExportPreview) -> ExportRequest {
    let planned: Vec<&ExportPreviewRow> = preview
        .rows
        .iter()
        .filter(|r| r.row.is_some() && r.identity.is_some())
        .collect();
    req.file_ids = planned.iter().map(|r| r.file_id).collect();
    req.confirmed = Some(ConfirmedExport {
        revision: preview.revision,
        files: planned
            .iter()
            .map(|r| ConfirmedFile {
                file_id: r.file_id,
                identity: r.identity.clone().expect("planned"),
                same_name: name_taken(r),
            })
            .collect(),
    });
    req
}

/// A tone of the same name in a folder of its own under the library.
fn tone_in(lib: &Lib, folder: &str, name: &str) -> PathBuf {
    let dir = lib.base().join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    tone_wav(&path, 0.2, 44_100);
    path
}

/// Every file under `dir` with its bytes' hash and modification time; folders by name.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Option<(Vec<u8>, std::time::SystemTime)>> {
    let mut found = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            found.insert(path.clone(), None);
            found.extend(tree(&path));
        } else if meta.is_file() {
            found.insert(
                path.clone(),
                Some((sha256(&path), meta.modified().unwrap())),
            );
        } else {
            found.insert(path, None);
        }
    }
    found
}

#[test]
fn regression_a_row_that_cannot_be_planned_takes_no_name() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let a = tone_in(&lib, "a", "track.wav");
    let b = tone_in(&lib, "b", "track.wav");
    let c = tone_in(&lib, "c", "other.wav");
    let ids = analysed(&shell, &[&a, &b, &c]);
    // `a` changes after its analysis, and another length tells it apart at once.
    let mut bytes = std::fs::read(&a).unwrap();
    bytes.extend_from_slice(&[0; 2]);
    std::fs::write(&a, &bytes).unwrap();

    let out = lib.base().join("out");
    let req = request(&ids, to_folder(), Some(&out));
    let preview = shell.plan_export(&req).unwrap();
    assert_eq!(
        preview.rows[0].error.as_ref().map(|e| e.kind),
        Some(IpcErrorKind::FileChanged)
    );
    assert!(writes(&preview.rows[1]), "{:?}", preview.rows[1]);
    assert!(writes(&preview.rows[2]), "{:?}", preview.rows[2]);

    // The job writes the two the confirm said it would.
    let (_, events) = export(&shell, confirmed(req, &preview));
    assert!(steps(&events, ids[1]).contains(&"exportDone"), "{events:?}");
    assert!(steps(&events, ids[2]).contains(&"exportDone"), "{events:?}");
    assert!(out.join("track.wav").exists());
    assert!(out.join("other.wav").exists());
}

#[test]
fn regression_a_row_that_stays_a_skip_takes_no_name() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let dir = lib.base().join("a");
    std::fs::create_dir_all(&dir).unwrap();
    let hi_res = dir.join("track.wav");
    tone_wav(&hi_res, 0.2, 96_000);
    let plain = tone_in(&lib, "b", "track.wav");
    let ids = analysed(&shell, &[&hi_res, &plain]);

    let out = lib.base().join("out");
    let req = request(&ids, to_folder(), Some(&out));
    let preview = shell.plan_export(&req).unwrap();
    assert_eq!(
        preview.rows[0].row.as_ref().unwrap().action,
        ExportAction::Skip {
            reason: ExportSkip::NotDjSafeRate {
                sample_rate_hz: 96_000
            }
        }
    );
    assert!(
        writes(&preview.rows[1]),
        "no copy of that name exists or is planned: {:?}",
        preview.rows[1]
    );

    let (_, events) = export(&shell, confirmed(req, &preview));
    assert!(steps(&events, ids[1]).contains(&"exportDone"), "{events:?}");
    assert!(out.join("track.wav").exists());
}

#[test]
fn of_the_rows_that_write_the_first_in_the_list_takes_the_name() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let first = tone_in(&lib, "a", "track.wav");
    let later = tone_in(&lib, "b", "track.wav");
    let ids = analysed(&shell, &[&first, &later]);
    let out = lib.base().join("out");
    let preview = shell
        .plan_export(&request(&ids, to_folder(), Some(&out)))
        .unwrap();
    assert!(writes(&preview.rows[0]));
    assert!(name_taken(&preview.rows[1]), "{:?}", preview.rows[1]);
    assert!(!preview.rows[1].replaces_copy);

    // The other way round, the other one does.
    let swapped = [ids[1], ids[0]];
    let preview = shell
        .plan_export(&request(&swapped, to_folder(), Some(&out)))
        .unwrap();
    assert_eq!(preview.rows[0].file_id, ids[1]);
    assert!(writes(&preview.rows[0]));
    assert!(name_taken(&preview.rows[1]));
}

#[test]
fn regression_a_row_the_confirm_skipped_stays_skipped_when_the_name_taker_drops_out() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let a = tone_in(&lib, "a", "track.wav");
    let b = tone_in(&lib, "b", "track.wav");
    let c = tone_in(&lib, "c", "other.wav");
    let ids = analysed(&shell, &[&a, &b, &c]);

    let out = lib.base().join("out");
    let req = request(&ids, to_folder(), Some(&out));
    let preview = shell.plan_export(&req).unwrap();
    assert!(name_taken(&preview.rows[1]));
    let mut sent = confirmed(req, &preview);
    // The row that took the name goes: recomputing would hand it to the row told it is skipped.
    sent.file_ids.retain(|id| *id != ids[0]);
    if let Some(c) = sent.confirmed.as_mut() {
        c.files.retain(|f| f.file_id != ids[0]);
    }
    let (_, events) = export(&shell, sent);

    let skipped = events.iter().find_map(|e| match e {
        JobEvent::ExportSkipped { file_id, row, .. } if *file_id == ids[1] => Some(row.clone()),
        _ => None,
    });
    assert_eq!(
        skipped.expect("skipped").action,
        ExportAction::Skip {
            reason: ExportSkip::SameNameInBatch
        }
    );
    assert!(steps(&events, ids[2]).contains(&"exportDone"), "{events:?}");
    assert!(!out.join("track.wav").exists(), "nothing took the name");
    assert!(out.join("other.wav").exists());
}

#[test]
fn a_name_taken_only_in_this_batch_says_so_and_what_to_do() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let a = tone_in(&lib, "a", "track.wav");
    let b = tone_in(&lib, "b", "track.wav");
    let ids = analysed(&shell, &[&a, &b]);
    let out = lib.base().join("out");
    let preview = shell
        .plan_export(&request(&ids, to_folder(), Some(&out)))
        .unwrap();
    assert_eq!(
        preview.rows[1].row.as_ref().unwrap().notes,
        [
            "a file earlier in this export takes this name in the folder; rename one, or choose \
          another folder"
        ]
    );
    assert!(
        !out.exists(),
        "the plan writes nothing, not even the folder"
    );
}

#[test]
fn regression_the_plan_says_what_an_export_in_place_would_refuse_and_changes_nothing() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let plain = lib.tone("plain.wav", 0.2);
    let target = lib.tone("target.wav", 0.2);
    let link = lib.music().join("link.wav");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let linked = lib.tone("linked.wav", 0.2);
    std::fs::hard_link(&linked, lib.base().join("second-name.wav")).unwrap();
    let read_only = lib.tone("read-only.wav", 0.2);

    let ids = analysed(&shell, &[&plain, &link, &linked, &read_only]);
    let mut perms = std::fs::metadata(&read_only).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&read_only, perms).unwrap();

    let before = tree(&lib.base());
    let req = request(&ids, in_place(), None);
    let preview = shell.plan_export(&req).unwrap();
    assert_eq!(tree(&lib.base()), before, "the plan writes nothing");
    assert!(!lib.backups().join("journal.jsonl").exists());

    assert!(writes(&preview.rows[0]), "{:?}", preview.rows[0]);
    for (row, what) in preview.rows[1..]
        .iter()
        .zip(["link", "hard link", "read-only"])
    {
        let error = row
            .error
            .as_ref()
            .unwrap_or_else(|| panic!("{what}: {row:?}"));
        assert_eq!(error.kind, IpcErrorKind::InPlaceRefused, "{what}");
        assert_eq!(error.file_id, Some(row.file_id), "{what}");
        assert!(
            row.row.is_none() && row.identity.is_none(),
            "{what}: {row:?}"
        );
    }

    // The job gets what the confirm said it could write, and writes it.
    let plain_before = sha256(&plain);
    let target_before = sha256(&target);
    let (_, events) = export(&shell, confirmed(req, &preview));
    assert!(steps(&events, ids[0]).contains(&"exportDone"), "{events:?}");
    assert_ne!(sha256(&plain), plain_before);
    assert_eq!(sha256(&target), target_before);
    assert!(
        !events.iter().any(|e| matches!(e, JobEvent::Failed { .. })),
        "{events:?}"
    );
}

// `chflags` and the flags it sets are macOS only.
#[cfg(target_os = "macos")]
#[test]
fn a_locked_file_is_refused_in_the_plan() {
    let run = |args: &[&str]| {
        assert!(
            std::process::Command::new(args[0])
                .args(&args[1..])
                .status()
                .unwrap()
                .success(),
            "{args:?} failed"
        );
    };
    let lib = Lib::new();
    let shell = lib.shell(1);
    let locked = lib.tone("locked.wav", 0.2);
    let ids = analysed(&shell, &[&locked]);
    let path = locked.to_str().unwrap();
    run(&["chflags", "uchg", path]);
    let preview = shell.plan_export(&request(&ids, in_place(), None));
    // Unlocked before anything is asserted: the temp folder has to go, whatever the result.
    run(&["chflags", "nouchg", path]);
    assert_eq!(
        preview.unwrap().rows[0].error.as_ref().map(|e| e.kind),
        Some(IpcErrorKind::InPlaceRefused)
    );
}

#[test]
fn a_link_that_goes_to_a_folder_is_not_refused() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let target = lib.tone("target.wav", 0.2);
    let link = lib.music().join("link.wav");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let ids = analysed(&shell, &[&link]);
    let out = lib.base().join("out");
    let preview = shell
        .plan_export(&request(&ids, to_folder(), Some(&out)))
        .unwrap();
    assert!(writes(&preview.rows[0]), "{:?}", preview.rows[0]);
}

#[test]
fn a_file_an_export_in_place_would_skip_anyway_is_not_refused() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let target = lib.tone_at("target.wav", 0.2, 96_000);
    let link = lib.music().join("link.wav");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let ids = analysed(&shell, &[&link]);
    let preview = shell.plan_export(&request(&ids, in_place(), None)).unwrap();
    assert!(
        matches!(
            preview.rows[0].row.as_ref().map(|r| &r.action),
            Some(ExportAction::Skip { .. })
        ),
        "{:?}",
        preview.rows[0]
    );
}

#[test]
fn a_moved_and_changed_source_is_told_true_of_the_copy_in_the_folder() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let a = tone_in(&lib, "a", "track.wav");
    let ids = analysed(&shell, &[&a]);
    let out = lib.base().join("out");
    let (_, events) = export(&shell, request(&ids, to_folder(), Some(&out)));
    assert!(steps(&events, ids[0]).contains(&"exportDone"), "{events:?}");

    // Moved, and changed the way new tags change it: not the bytes the copy was made from.
    let moved_dir = lib.base().join("moved");
    std::fs::create_dir_all(&moved_dir).unwrap();
    let moved = moved_dir.join("track.wav");
    std::fs::rename(&a, &moved).unwrap();
    let mut bytes = std::fs::read(&moved).unwrap();
    bytes.extend_from_slice(&[0; 2]);
    std::fs::write(&moved, &bytes).unwrap();
    let again = analysed(&shell, &[&moved]);

    let preview = shell
        .plan_export(&request(&again, to_folder(), Some(&out)))
        .unwrap();
    let row = preview.rows[0].row.as_ref().unwrap();
    assert_eq!(
        row.action,
        ExportAction::Skip {
            reason: ExportSkip::CopyOfOtherTrack
        }
    );
    assert_eq!(
        row.notes,
        [
            "the folder has a copy of this name that SoundCheck made from another file, or from \
             this one before it was moved and changed; rename one, or choose another folder"
        ]
    );
}

fn at_export_done(e: &JobEvent) -> bool {
    matches!(e, JobEvent::ExportDone { .. })
}

#[test]
fn a_client_that_says_no_name_is_taken_still_overwrites_nothing() {
    let lib = Lib::new();
    let shell = lib.shell(1);
    let a = tone_in(&lib, "a", "track.wav");
    let b = tone_in(&lib, "b", "track.wav");
    // Another level, so the two copies could not pass for each other.
    tone_wav(&b, 0.4, 44_100);
    let ids = analysed(&shell, &[&a, &b]);
    let out = lib.base().join("out");
    let req = request(&ids, to_folder(), Some(&out));
    let preview = shell.plan_export(&req).unwrap();
    let mut sent = confirmed(req, &preview);
    sent.file_ids = ids.clone();
    if let Some(c) = sent.confirmed.as_mut() {
        for f in &mut c.files {
            f.same_name = false;
        }
        assert_eq!(c.files.len(), 2, "both rows confirmed");
    }

    // Held right after the first copy is written: its bytes then are the ones to keep.
    let held = super::support::held_at(&shell, sent, at_export_done);
    let copy = out.join("track.wav");
    let first = sha256(&copy);
    let events = held.finish();

    assert!(steps(&events, ids[0]).contains(&"exportDone"), "{events:?}");
    let second = steps(&events, ids[1]);
    assert!(!second.contains(&"exportDone"), "{events:?}");
    assert!(
        second.contains(&"exportSkipped") || second.contains(&"failed"),
        "{events:?}"
    );
    assert_eq!(sha256(&copy), first, "the first copy is not written over");
    let names: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().ends_with(".wav"))
        .collect();
    assert_eq!(names, ["track.wav"], "one copy, no second name made up");
}
