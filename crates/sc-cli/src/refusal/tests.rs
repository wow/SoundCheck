//! Unit tests of `refusal.rs`: the three lines for the refusals the CLI can meet.

use std::path::PathBuf;

use super::*;

fn lines(err: &Error, action: Action) -> String {
    let mut out = Vec::new();
    write(
        &mut out,
        &PathBuf::from("/Music/Track.wav"),
        action,
        &explain(err, action),
    )
    .expect("written");
    String::from_utf8(out).expect("utf-8")
}

#[test]
fn every_refusal_is_three_lines_naming_the_file() {
    let path = PathBuf::from("/Music/Track.wav");
    let errors = [
        Error::RekordboxUsbExport {
            path: path.clone(),
            volume: "/Volumes/USB".into(),
        },
        Error::InPlaceRefused {
            path: path.clone(),
            reason: InPlaceRefusal::Symlink,
        },
        Error::NoSpace {
            volume: "/".into(),
            needed_bytes: 1_500_000_000,
            free_bytes: 200_000_000,
        },
        Error::NotDjSafe {
            path: path.clone(),
            reason: "96000 Hz".into(),
        },
        Error::WouldClip {
            needed_db: 3.0,
            over_db: 1.234,
        },
        Error::FileChanged {
            path: path.clone(),
            detail: "while SoundCheck was processing it".into(),
        },
        Error::UnsupportedFormat {
            path: path.clone(),
            detail: "MP3".into(),
        },
        Error::NothingToUndo { path: path.clone() },
        Error::AlreadyExists { path },
        Error::Cancelled,
    ];
    for err in &errors {
        let text = lines(err, Action::Change);
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows.len(), 3, "{text}");
        assert_eq!(rows[0], "Track.wav: not changed");
        assert!(
            rows[1].starts_with("  why: ") && rows[1].len() > 9,
            "{text}"
        );
        assert!(
            rows[2].starts_with("  what to do: ") && rows[2].len() > 16,
            "{text}"
        );
    }
}

#[test]
fn the_advice_is_specific() {
    let r = explain(
        &Error::NoSpace {
            volume: "/".into(),
            needed_bytes: 1_500_000_000,
            free_bytes: 200_000_000,
        },
        Action::Change,
    );
    assert_eq!(
        r.why,
        "not enough free space on /: 1500.0 MB needed (64 MiB margin included), 200.0 MB free"
    );
    let r = explain(
        &Error::WouldClip {
            needed_db: 3.0,
            over_db: 1.234,
        },
        Action::Change,
    );
    assert_eq!(r.what_to_do, "use --gain-db +1.75 or lower");
    let r = explain(
        &Error::InPlaceRefused {
            path: "a.wav".into(),
            reason: InPlaceRefusal::HardLinked { links: 3 },
        },
        Action::Change,
    );
    assert!(r.why.contains("3 hard links"), "{}", r.why);
    assert!(r.what_to_do.contains("--out"), "{}", r.what_to_do);
    assert!(
        lines(
            &Error::NothingToUndo {
                path: "a.wav".into()
            },
            Action::Undo
        )
        .starts_with("Track.wav: not undone\n")
    );
    assert!(lines(&Error::Cancelled, Action::Copy).starts_with("Track.wav: not written\n"));
}

#[test]
fn the_suggested_gain_stays_below_the_clip_point() {
    for (needed, over) in [(3.0, 1.234), (0.5, 0.0), (6.0, 6.0), (-1.0, 0.004)] {
        let max = max_gain_db(needed, over);
        assert!(max < needed - over, "{needed} {over} -> {max}");
        assert!(max > needed - over - 0.02, "{needed} {over} -> {max}");
    }
}
