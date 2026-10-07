//! Inputs checked together before anything is written: one file listed twice under different
//! spellings (applied once), copies into one `--out` folder that would share a name (only the
//! first written), and a duplicate in `undo`.

mod common;

use common::{Library, arg, wav_bytes};

#[test]
fn a_file_listed_twice_is_applied_once() {
    let lib = Library::new();
    let wav = lib.file("T.wav", &wav_bytes(false));
    let mut args = vec!["apply", "T.wav", "./T.wav", arg(&wav)];
    // macOS volumes ignore case, so `t.wav` is the same file there.
    let folds_case = lib.music.join("t.wav").exists();
    if folds_case {
        args.push("t.wav");
    }
    args.extend(["--gain-db", "-3"]);
    let run = lib.run_in(&lib.music, &args);
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert_eq!(
        run.stdout.lines().count(),
        1,
        "applied once: {}",
        run.text()
    );
    assert!(
        run.stdout.starts_with("T.wav: gain -3.00 dB"),
        "{}",
        run.text()
    );
    let mut want = format!(
        "./T.wav: not changed\n  why: it is listed twice: it is the same file as T.wav\n  \
         what to do: nothing to do: it is processed once, for its first mention\n\
         {}: not changed\n  why: it is listed twice: it is the same file as T.wav\n  \
         what to do: nothing to do: it is processed once, for its first mention\n",
        wav.display()
    );
    if folds_case {
        want.push_str(
            "t.wav: not changed\n  why: it is listed twice: it is the same file as T.wav\n  \
             what to do: nothing to do: it is processed once, for its first mention\n",
        );
    }
    assert_eq!(run.stderr, want);
    // One change journaled, one backup.
    let run = lib.run(&["journal", "--json"]);
    assert_eq!(run.docs()[0]["entries"].as_array().map(Vec::len), Some(1));

    let run = lib.run_in(&lib.music, &["undo", "T.wav", "./T.wav", "--json"]);
    assert_eq!(run.code, Some(2), "{}", run.text());
    let docs = run.docs();
    assert_eq!(docs[0]["ok"], true);
    assert_eq!(docs[1]["ok"], false);
    assert_eq!(docs[1]["error"]["kind"], "listedTwice");
    assert_eq!(std::fs::read(&wav).expect("restored"), wav_bytes(false));
}

#[test]
fn copies_that_would_share_a_name_are_refused_before_anything_is_written() {
    let lib = Library::new();
    let a = lib.music.join("a");
    let b = lib.music.join("b");
    std::fs::create_dir_all(&a).expect("a");
    std::fs::create_dir_all(&b).expect("b");
    std::fs::write(a.join("Track.wav"), wav_bytes(false)).expect("a/Track.wav");
    std::fs::write(b.join("TRACK.wav"), wav_bytes(false)).expect("b/TRACK.wav");
    let out = lib.base.join("out");
    let run = lib.run_in(
        &lib.music,
        &[
            "apply",
            "a/Track.wav",
            "b/TRACK.wav",
            "--gain-db",
            "-1",
            "--out",
            arg(&out),
        ],
    );
    assert_eq!(run.code, Some(2), "{}", run.text());
    assert!(
        run.stdout.starts_with("a/Track.wav: gain -1.00 dB"),
        "{}",
        run.text()
    );
    assert_eq!(
        run.stderr,
        format!(
            "b/TRACK.wav: not written\n  why: a/Track.wav would be written as {} too\n  \
             what to do: rename one of them, or run it again with another --out folder\n",
            out.join("TRACK.wav").display()
        )
    );
    let mut written: Vec<String> = std::fs::read_dir(&out)
        .expect("out")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(written, vec!["Track.wav", "Track.wav.soundcheck.json"]);
}
