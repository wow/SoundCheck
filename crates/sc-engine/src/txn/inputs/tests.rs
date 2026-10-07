//! Unit tests of `inputs.rs`: one file under several spellings, output name collisions.

use std::path::Path;

use super::*;

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    std::fs::write(path, b"x").expect("file");
}

fn kinds(found: &[Option<Error>]) -> Vec<&'static str> {
    found
        .iter()
        .map(|e| match e {
            None => "ok",
            Some(Error::ListedTwice { .. }) => "twice",
            Some(Error::SameOutputName { .. }) => "same name",
            Some(_) => "other",
        })
        .collect()
}

#[test]
fn a_file_listed_under_several_spellings_is_refused_after_its_first_mention() {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let file = base.join("music/T.wav");
    touch(&file);
    let dotted = base.join("music/./T.wav");
    let up = base.join("music/../music/T.wav");
    let missing = base.join("music/gone.wav");
    let mut files = vec![file.clone(), dotted.clone(), up, missing];
    // Another spelling the file system folds onto the same file (macOS volumes ignore case).
    let upper = base.join("music/t.wav");
    let folds_case = upper.exists();
    if folds_case {
        files.push(upper);
    }
    let found = check_inputs(&files, &Place::InPlace);
    let mut want = vec!["ok", "twice", "twice", "ok"];
    if folds_case {
        want.push("twice");
    }
    assert_eq!(kinds(&found), want);
    match &found[1] {
        Some(Error::ListedTwice { path, first }) => {
            assert_eq!((path, first), (&dotted, &file));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn copies_into_one_folder_may_not_share_a_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.path().canonicalize().expect("resolves");
    let (a, b, c) = (
        base.join("a/Track.wav"),
        base.join("b/TRACK.wav"),
        base.join("c/Other.wav"),
    );
    for f in [&a, &b, &c] {
        touch(f);
    }
    let out = base.join("out");
    let files = vec![a.clone(), b.clone(), c, a.clone()];
    let found = check_inputs(&files, &Place::Folder(out.clone()));
    assert_eq!(kinds(&found), vec!["ok", "same name", "ok", "twice"]);
    match &found[1] {
        Some(Error::SameOutputName {
            path,
            other,
            output,
        }) => {
            assert_eq!((path, other), (&b, &a));
            assert_eq!(output, &out.join("TRACK.wav"));
        }
        other => panic!("{other:?}"),
    }
    // In place there is no shared folder.
    assert_eq!(
        kinds(&check_inputs(&files[..3], &Place::InPlace)),
        vec!["ok", "ok", "ok"]
    );
}
