//! Why a file was not changed, in three lines: what happened, why, and what to do.

use std::io::Write;
use std::path::Path;

use sc_core::{Error, InPlaceRefusal};
use serde::Serialize;

/// The command a refusal belongs to: it decides the first line and some of the advice.
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

const OR_COPY: &str = "or write a copy with --out <folder>";

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

fn in_place(reason: InPlaceRefusal) -> Refusal {
    match reason {
        InPlaceRefusal::Symlink => refusal(
            "it is a symbolic link",
            format!("apply to the file it points to, {OR_COPY}"),
        ),
        InPlaceRefusal::FinderLocked => refusal(
            "it is locked (Finder: Get Info > Locked)",
            format!("unlock it, {OR_COPY}"),
        ),
        InPlaceRefusal::HasAcl => refusal(
            "it has an access control list, which the new version would lose",
            format!("remove the list (chmod -N), {OR_COPY}"),
        ),
        InPlaceRefusal::ReadOnlyFile => refusal(
            "it is read-only",
            format!(
                "allow writing (chmod u+w, or Finder: Get Info > Sharing & Permissions), {OR_COPY}"
            ),
        ),
        InPlaceRefusal::ReadOnlyFolder => refusal(
            "its folder is read-only or locked",
            format!("allow writing to the folder, {OR_COPY}"),
        ),
        InPlaceRefusal::OwnedByOtherUser => refusal(
            "it belongs to another user, and the new version would belong to you",
            format!("process it as its owner, {OR_COPY}"),
        ),
        InPlaceRefusal::HardLinked { links } => refusal(
            format!("it has {links} hard links, which replacing it would break"),
            "write a copy with --out <folder>",
        ),
        other => refusal(other.to_string(), "write a copy with --out <folder>"),
    }
}

/// The largest gain below `needed_db` that keeps a peak `over_db` above full scale under it,
/// rounded down to 0.01 dB.
fn max_gain_db(needed_db: f64, over_db: f64) -> f64 {
    ((needed_db - over_db.max(0.0)) * 100.0 - 1.0).floor() / 100.0
}

/// Why `err` stopped `action`, and what to do.
#[must_use]
pub fn explain(err: &Error, action: Action) -> Refusal {
    match err {
        Error::RekordboxUsbExport { volume, .. } => refusal(
            format!(
                "it is on a rekordbox USB export ({}), where rekordbox keeps its own analysis",
                volume.display()
            ),
            "process the file in your library, then export again from rekordbox",
        ),
        Error::InPlaceRefused { reason, .. } => in_place(*reason),
        Error::NoSpace {
            volume,
            needed_bytes,
            free_bytes,
        } => refusal(
            format!(
                "not enough free space on {}: {} needed (64 MiB margin included), {} free",
                volume.display(),
                mb(*needed_bytes),
                mb(*free_bytes)
            ),
            "free some space, or choose another --backup-root or --out folder",
        ),
        Error::NotDjSafe { reason, .. } => refusal(
            format!("the output would not be DJ-safe: {reason}"),
            "convert the file to 44.1 or 48 kHz (and under 2 GiB for AIFF, 4 GiB for WAV) first",
        ),
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
        Error::FileChanged { detail, .. } => refusal(
            format!("it changed {detail}"),
            match action {
                Action::Undo => "keep it as it is, or copy the original back from the backup",
                Action::Change | Action::Copy => "run the command again",
            },
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
                "the new version did not read back as written ({detail}); the original is \
                 untouched"
            ),
            "run the command again; if it fails again, report it with the file",
        ),
        Error::AlreadyExists { path } => refusal(
            format!(
                "{} already exists, and SoundCheck never overwrites a file it did not back up",
                path.display()
            ),
            "choose another --out folder, or move that file away",
        ),
        Error::NothingToUndo { .. } => refusal(
            "no change of it is recorded in the backup folder",
            "check --backup-root, or list the changes with sc-cli journal",
        ),
        Error::InvalidArgument(msg) => refusal(msg.clone(), "check the command's arguments"),
        Error::Io { path, source } => refusal(
            format!("{source} ({})", path.display()),
            "check that the file and its folders are reachable, then run the command again",
        ),
        other => refusal(other.to_string(), "run the command again"),
    }
}

/// The name a line starts with: the file name, or the whole path when it has none.
#[must_use]
pub fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Writes the three lines for `file`.
///
/// # Errors
/// Whatever `out` returns.
pub fn write(
    out: &mut impl Write,
    file: &Path,
    action: Action,
    refusal: &Refusal,
) -> std::io::Result<()> {
    writeln!(out, "{}: {}", name(file), action.what())?;
    writeln!(out, "  why: {}", refusal.why)?;
    writeln!(out, "  what to do: {}", refusal.what_to_do)
}

#[cfg(test)]
mod tests;
