//! Why a file was not changed, in three lines: what happened, why, and what to do. The advice
//! depends on the command: `apply` in place may point to `--out`; `apply --out` never blames the
//! source for its destination; `undo` never suggests `--out` or `--backup-root`.

use std::io::Write;
use std::path::Path;

use sc_core::{ChangeCause, Error, InPlaceRefusal};
use serde::Serialize;

/// The command a refusal belongs to: it decides the first line and the advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `apply` in place.
    Change,
    /// `apply --out`.
    Copy,
    /// `undo`.
    Undo,
}

impl Action {
    fn what(self) -> &'static str {
        match self {
            Self::Change => "not changed",
            Self::Copy => "not written",
            Self::Undo => "not undone",
        }
    }
}

/// Why, and what to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    /// Why the file was not changed.
    pub why: String,
    /// What the user can do about it.
    pub what_to_do: String,
}

fn refusal(why: impl Into<String>, what_to_do: impl Into<String>) -> Refusal {
    Refusal {
        why: why.into(),
        what_to_do: what_to_do.into(),
    }
}

/// Bytes as megabytes (10^6), one decimal.
fn mb(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // display only
    let mb = bytes as f64 / 1e6;
    format!("{mb:.1} MB")
}

/// `fix`, then for `apply` in place the alternative of a copy; for `undo` only `fix`.
fn fix_or_copy(fix: &str, action: Action) -> String {
    match action {
        Action::Change => format!("{fix}, or write a copy with --out <folder>"),
        Action::Copy | Action::Undo => fix.to_owned(),
    }
}

fn in_place(reason: InPlaceRefusal, action: Action) -> Refusal {
    let again = match action {
        Action::Undo => "then run undo again",
        Action::Change | Action::Copy => "then run the command again",
    };
    match reason {
        InPlaceRefusal::Symlink => refusal(
            "it is a symbolic link",
            fix_or_copy("name the file it points to instead", action),
        ),
        InPlaceRefusal::FinderLocked => refusal(
            "it is locked (Finder: Get Info > Locked)",
            fix_or_copy(&format!("unlock it, {again}"), action),
        ),
        InPlaceRefusal::HasAcl => refusal(
            "it has an access control list, which the new version would lose",
            fix_or_copy(&format!("remove the list (chmod -N), {again}"), action),
        ),
        InPlaceRefusal::ReadOnlyFile => refusal(
            "it is read-only",
            fix_or_copy(
                &format!(
                    "allow writing (chmod u+w, or Finder: Get Info > Sharing & Permissions), \
                     {again}"
                ),
                action,
            ),
        ),
        InPlaceRefusal::ReadOnlyFolder => refusal(
            "its folder is read-only or locked",
            fix_or_copy(&format!("allow writing to the folder, {again}"), action),
        ),
        InPlaceRefusal::OwnedByOtherUser => refusal(
            "it belongs to another user, and the new version would belong to you",
            fix_or_copy("process it as its owner", action),
        ),
        InPlaceRefusal::HardLinked { links } => refusal(
            format!("it has {links} hard links, which replacing it would break"),
            match action {
                Action::Undo => "remove the other links, then run undo again".to_owned(),
                Action::Change | Action::Copy => "write a copy with --out <folder>".to_owned(),
            },
        ),
        other => refusal(other.to_string(), fix_or_copy("fix that", action)),
    }
}

/// The largest gain below `needed_db` that keeps a peak `over_db` above full scale under it,
/// rounded down to 0.01 dB.
fn max_gain_db(needed_db: f64, over_db: f64) -> f64 {
    ((needed_db - over_db.max(0.0)) * 100.0 - 1.0).floor() / 100.0
}

fn changed(cause: ChangeCause, detail: &str, action: Action) -> Refusal {
    let why = format!("it changed {detail}");
    match cause {
        ChangeCause::OtherChangeFirst => refusal(
            why,
            "see sc-cli journal; if that change was yours, the file is done",
        ),
        ChangeCause::SincePlanned => refusal(
            why,
            "run the export again: it measures and plans the file as it is now",
        ),
        ChangeCause::SinceProcessed => refusal(
            why,
            "keep it as it is, or copy the original back from the backup by hand",
        ),
        _ => refusal(
            why,
            match action {
                Action::Undo => "close the app that writes to it, then run undo again",
                Action::Change | Action::Copy => {
                    "close the app that writes to it, then run the command again"
                }
            },
        ),
    }
}

fn rekordbox(volume: &Path, action: Action) -> Refusal {
    match action {
        Action::Copy => refusal(
            format!(
                "the --out folder is on a rekordbox USB export ({}), where rekordbox keeps its \
                 own analysis",
                volume.display()
            ),
            "choose an --out folder in your library, then export again from rekordbox",
        ),
        Action::Change | Action::Undo => refusal(
            format!(
                "it is on a rekordbox USB export ({}), where rekordbox keeps its own analysis",
                volume.display()
            ),
            match action {
                Action::Undo => "undo the change of the file in your library, then export again",
                Action::Change | Action::Copy => {
                    "process the file in your library, then export again from rekordbox"
                }
            },
        ),
    }
}

fn no_space(volume: &Path, needed: u64, free: u64, action: Action) -> Refusal {
    refusal(
        format!(
            "not enough free space on {}: {} needed (64 MiB margin included), {} free",
            volume.display(),
            mb(needed),
            mb(free)
        ),
        match action {
            Action::Change => "free some space, or choose another --backup-root or --out folder",
            Action::Copy => "free some space, or choose another --out folder",
            Action::Undo => "free some space on that volume, then run undo again",
        },
    )
}

fn not_dj_safe(reason: &str) -> Refusal {
    let what_to_do = if reason.contains("sample rate") {
        "convert it to 44.1 or 48 kHz in an audio editor first"
    } else {
        "it is too long for a WAV or AIFF file; shorten it in an audio editor first"
    };
    refusal(
        format!("the output would not be DJ-safe: {reason}"),
        what_to_do,
    )
}

/// Why `err` stopped `action`, and what to do.
#[must_use]
pub fn explain(err: &Error, action: Action) -> Refusal {
    match err {
        Error::RekordboxUsbExport { volume, .. } => rekordbox(volume, action),
        Error::InPlaceRefused { reason, .. } => in_place(*reason, action),
        Error::NoSpace {
            volume,
            needed_bytes,
            free_bytes,
        } => no_space(volume, *needed_bytes, *free_bytes, action),
        Error::NotDjSafe { reason, .. } => not_dj_safe(reason),
        Error::WouldClip { needed_db, over_db } => refusal(
            format!(
                "a gain of {needed_db:+.2} dB would take the peak to {over_db:+.2} dBFS, and \
                 SoundCheck never clips"
            ),
            format!(
                "use --gain-db {:+.2} or lower",
                max_gain_db(*needed_db, *over_db)
            ),
        ),
        Error::FileChanged { detail, cause, .. } => changed(*cause, detail, action),
        Error::ListedTwice { first, .. } => refusal(
            format!(
                "it is listed twice: it is the same file as {}",
                first.display()
            ),
            "nothing to do: it is processed once, for its first mention",
        ),
        Error::SameOutputName { other, output, .. } => refusal(
            format!(
                "{} would be written as {} too",
                other.display(),
                output.display()
            ),
            "rename one of them, or run it again with another --out folder",
        ),
        Error::UnsupportedFormat { detail, .. } => refusal(
            format!("its format is not supported for writing: {detail}"),
            "apply works on WAV, AIFF and FLAC files",
        ),
        Error::UnsupportedChannels { channels, .. } => refusal(
            format!("it has {channels} channels; only mono and stereo are supported"),
            "export a stereo version first",
        ),
        Error::Corrupt { detail, .. } => refusal(
            format!("it could not be read: {detail}"),
            "check that it plays, or export it again",
        ),
        Error::VerifyFailed { detail, .. } => refusal(
            format!(
                "the new version did not read back as written ({detail}); the file is \
                 untouched"
            ),
            "run the command again; if it fails again, report it with the file",
        ),
        Error::AlreadyExists { path } => refusal(
            format!(
                "{} already exists, and SoundCheck never overwrites a file it did not back up",
                path.display()
            ),
            match action {
                Action::Undo => "move that file away, then run undo again",
                Action::Change | Action::Copy => {
                    "choose another --out folder, or move that file away"
                }
            },
        ),
        Error::NothingToUndo { .. } => refusal(
            "no change of it is recorded in the backup folder",
            "list the recorded changes with sc-cli journal",
        ),
        Error::InvalidArgument(msg) => refusal(msg.clone(), "check the command's arguments"),
        Error::Io { path, source } => refusal(
            format!("{source} ({})", path.display()),
            "check that the file and its folders are reachable, then run the command again",
        ),
        other => refusal(other.to_string(), "run the command again"),
    }
}

/// Writes the three lines for `file`, named as given.
///
/// # Errors
/// Whatever `out` returns.
pub fn write(
    out: &mut impl Write,
    file: &Path,
    action: Action,
    refusal: &Refusal,
) -> std::io::Result<()> {
    writeln!(out, "{}: {}", file.display(), action.what())?;
    writeln!(out, "  why: {}", refusal.why)?;
    writeln!(out, "  what to do: {}", refusal.what_to_do)
}

#[cfg(test)]
mod tests;
