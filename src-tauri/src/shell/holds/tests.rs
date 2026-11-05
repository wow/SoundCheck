//! Unit tests of `src-tauri/src/shell/holds.rs`: who may share a file, and that a hold ends on
//! every way out.

use std::path::Path;
use std::sync::Arc;

use sc_core::ipc::IpcErrorKind;

use super::{HoldKind, Holds};

const A: (u32, &str) = (1, "/nowhere/a.wav");
const B: (u32, &str) = (2, "/nowhere/b.wav");

fn take(holds: &Arc<Holds>, file: (u32, &str), kind: HoldKind) -> Result<super::Hold, u32> {
    holds
        .acquire(&[(file.0, Path::new(file.1))], kind)
        .map_err(|e| {
            assert_eq!(e.kind, IpcErrorKind::Busy);
            e.file_id.expect("names the file")
        })
}

#[test]
fn an_export_or_a_revert_needs_the_file_to_itself() {
    use HoldKind::{ExportCopy, ExportInPlace, Reading, Revert};
    for held in [Reading, ExportCopy, ExportInPlace, Revert] {
        for wanted in [ExportCopy, ExportInPlace, Revert] {
            let holds = Arc::new(Holds::default());
            let hold = take(&holds, A, held).unwrap();
            assert_eq!(
                take(&holds, A, wanted).unwrap_err(),
                1,
                "{held:?} {wanted:?}"
            );
            drop(take(&holds, B, wanted).unwrap());
            drop(hold);
            drop(take(&holds, A, wanted).unwrap());
        }
    }
}

#[test]
fn analyses_share_a_file_but_not_with_an_export_or_a_revert() {
    let holds = Arc::new(Holds::default());
    let first = take(&holds, A, HoldKind::Reading).unwrap();
    let second = take(&holds, A, HoldKind::Reading).unwrap();
    drop(first);
    assert_eq!(
        take(&holds, A, HoldKind::Revert).unwrap_err(),
        1,
        "one left"
    );
    drop(second);
    for worker in [
        HoldKind::ExportCopy,
        HoldKind::ExportInPlace,
        HoldKind::Revert,
    ] {
        let hold = take(&holds, A, worker).unwrap();
        assert_eq!(
            take(&holds, A, HoldKind::Reading).unwrap_err(),
            1,
            "{worker:?}"
        );
        drop(hold);
    }
    assert!(holds.files().is_empty(), "nothing is left behind");
}

#[test]
fn all_files_are_held_or_none() {
    let holds = Arc::new(Holds::default());
    let on_b = take(&holds, B, HoldKind::ExportInPlace).unwrap();
    let both = [(A.0, Path::new(A.1)), (B.0, Path::new(B.1))];
    let refused = holds.acquire(&both, HoldKind::ExportInPlace).unwrap_err();
    assert_eq!(refused.file_id, Some(2));
    drop(take(&holds, A, HoldKind::Revert).expect("A was not kept"));
    drop(on_b);
}

#[test]
fn only_a_file_that_changes_refuses_a_grid_view() {
    let holds = Arc::new(Holds::default());
    let path = Path::new(A.1);
    for (kind, changes) in [
        (HoldKind::Reading, false),
        (HoldKind::ExportCopy, false),
        (HoldKind::ExportInPlace, true),
        (HoldKind::Revert, true),
    ] {
        let hold = take(&holds, A, kind).unwrap();
        assert_eq!(holds.refuse_changing(1, path).is_err(), changes, "{kind:?}");
        drop(hold);
    }
    assert!(holds.refuse_changing(1, path).is_ok());
}

#[test]
fn a_panic_releases_the_hold() {
    let holds = Arc::new(Holds::default());
    let thread_holds = Arc::clone(&holds);
    let joined = std::thread::spawn(move || {
        let _hold = take(&thread_holds, A, HoldKind::ExportInPlace).unwrap();
        panic!("the job broke");
    })
    .join();
    assert!(joined.is_err());
    assert!(holds.files().is_empty());
}

#[test]
fn two_spellings_of_a_file_are_one_hold() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("a.wav");
    std::fs::write(&path, b"x").unwrap();
    let spelled = path.parent().unwrap().join("./a.wav");
    let holds = Arc::new(Holds::default());
    let hold = holds.acquire(&[(1, &path)], HoldKind::Revert).unwrap();
    assert!(holds.acquire(&[(2, &spelled)], HoldKind::Revert).is_err());
    drop(hold);
}
