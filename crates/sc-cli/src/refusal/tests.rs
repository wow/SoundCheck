//! Unit tests of `refusal.rs`: the three lines, and advice that fits the command and the cause.

use std::path::PathBuf;

use super::*;

fn lines(err: &Error, action: Action) -> String {
    let mut out = Vec::new();
    write(
        &mut out,
        &PathBuf::from("Music/Track.wav"),
        action,
        &explain(err, action),
    )
    .expect("written");
    String::from_utf8(out).expect("utf-8")
}

fn path() -> PathBuf {
    PathBuf::from("/Music/Track.wav")
}

fn errors() -> Vec<Error> {
    vec![
        Error::RekordboxUsbExport {
            path: path(),
            volume: "/Volumes/USB".into(),
        },
        Error::InPlaceRefused {
            path: path(),
            reason: InPlaceRefusal::Symlink,
        },
        Error::InPlaceRefused {
            path: path(),
            reason: InPlaceRefusal::ReadOnlyFile,
        },
        Error::InPlaceRefused {
            path: path(),
            reason: InPlaceRefusal::HardLinked { links: 2 },
        },
        Error::NoSpace {
            volume: "/".into(),
            needed_bytes: 1_500_000_000,
            free_bytes: 200_000_000,
        },
        Error::NotDjSafe {
            path: path(),
            reason: "sample rate 96000 Hz; DJ-safe outputs are 44,100 or 48,000 Hz".into(),
        },
        Error::WouldClip {
            needed_db: 3.0,
            over_db: 1.234,
        },
        Error::FileChanged {
            path: path(),
            detail: "while SoundCheck was processing it".into(),
            cause: ChangeCause::DuringProcessing,
        },
        Error::ListedTwice {
            path: "./Track.wav".into(),
            first: "Track.wav".into(),
        },
        Error::UnsupportedFormat {
            path: path(),
            detail: "MP3".into(),
        },
        Error::NothingToUndo { path: path() },
        Error::AlreadyExists { path: path() },
        Error::Cancelled,
    ]
}

#[test]
fn every_refusal_is_three_lines_naming_the_file_as_given() {
    for action in [Action::Change, Action::Copy, Action::Undo] {
        for err in &errors() {
            let text = lines(err, action);
            let rows: Vec<&str> = text.lines().collect();
            assert_eq!(rows.len(), 3, "{text}");
            assert_eq!(rows[0], format!("Music/Track.wav: {}", action.what()));
            assert!(
                rows[1].starts_with("  why: ") && rows[1].len() > 9,
                "{text}"
            );
            assert!(
                rows[2].starts_with("  what to do: ") && rows[2].len() > 16,
                "{text}"
            );
            if action == Action::Undo {
                assert!(
                    !rows[2].contains("--out") && !rows[2].contains("--backup-root"),
                    "undo advice: {text}"
                );
            }
            assert!(
                !rows[1].contains("Try again") && !rows[1].contains("try again"),
                "{text}"
            );
        }
    }
}

#[test]
fn the_advice_fits_the_command() {
    let usb = Error::RekordboxUsbExport {
        path: "/Volumes/USB/out".into(),
        volume: "/Volumes/USB".into(),
    };
    let copy = explain(&usb, Action::Copy);
    assert!(
        copy.why
            .starts_with("the --out folder is on a rekordbox USB export"),
        "{copy:?}"
    );
    assert!(
        copy.what_to_do.starts_with("choose an --out folder"),
        "{copy:?}"
    );
    assert!(
        explain(&usb, Action::Change)
            .why
            .starts_with("it is on a rekordbox USB export")
    );

    let ro = Error::InPlaceRefused {
        path: path(),
        reason: InPlaceRefusal::ReadOnlyFile,
    };
    assert!(
        explain(&ro, Action::Change)
            .what_to_do
            .ends_with("or write a copy with --out <folder>")
    );
    assert!(
        explain(&ro, Action::Undo)
            .what_to_do
            .ends_with("then run undo again")
    );

    let space = Error::NoSpace {
        volume: "/".into(),
        needed_bytes: 1_500_000_000,
        free_bytes: 200_000_000,
    };
    let r = explain(&space, Action::Change);
    assert_eq!(
        r.why,
        "not enough free space on /: 1500.0 MB needed (64 MiB margin included), 200.0 MB free"
    );
    assert!(r.what_to_do.contains("--backup-root"));
    assert!(
        !explain(&space, Action::Copy)
            .what_to_do
            .contains("--backup-root")
    );
}

#[test]
fn the_advice_fits_the_cause() {
    let rate = Error::NotDjSafe {
        path: path(),
        reason: "sample rate 96000 Hz; DJ-safe outputs are 44,100 or 48,000 Hz".into(),
    };
    assert!(
        explain(&rate, Action::Change)
            .what_to_do
            .contains("44.1 or 48 kHz")
    );
    let size = Error::NotDjSafe {
        path: path(),
        reason: "the output would hold 5000000000 bytes, past the 4 GiB a WAV file can address"
            .into(),
    };
    assert!(
        explain(&size, Action::Change)
            .what_to_do
            .contains("too long")
    );

    let first = Error::FileChanged {
        path: path(),
        detail: "while SoundCheck was waiting to process it (another change of it ran first)"
            .into(),
        cause: ChangeCause::OtherChangeFirst,
    };
    assert_eq!(
        explain(&first, Action::Change).what_to_do,
        "see sc-cli journal; if that change was yours, the file is done"
    );
    let clip = explain(
        &Error::WouldClip {
            needed_db: 3.0,
            over_db: 1.234,
        },
        Action::Change,
    );
    assert_eq!(clip.what_to_do, "use --gain-db +1.75 or lower");
    let same = explain(
        &Error::SameOutputName {
            path: "b/Track.wav".into(),
            other: "a/Track.wav".into(),
            output: "out/Track.wav".into(),
        },
        Action::Copy,
    );
    assert_eq!(
        same.why,
        "a/Track.wav would be written as out/Track.wav too"
    );
}

#[test]
fn the_suggested_gain_stays_below_the_clip_point() {
    for (needed, over) in [(3.0, 1.234), (0.5, 0.0), (6.0, 6.0), (-1.0, 0.004)] {
        let max = max_gain_db(needed, over);
        assert!(max < needed - over, "{needed} {over} -> {max}");
        assert!(max > needed - over - 0.02, "{needed} {over} -> {max}");
    }
}
