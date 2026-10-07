//! One error variant per failure class the UI must distinguish.

use std::path::PathBuf;

use crate::ipc::IpcErrorKind;

/// Failure classes. Library crates return these; binaries wrap them in `anyhow`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The container or codec is not supported (or not supported for writing).
    #[error("unsupported format for {}: {detail}", path.display())]
    UnsupportedFormat {
        /// The file concerned.
        path: PathBuf,
        /// What was not supported.
        detail: String,
    },
    /// The file has more channels than SoundCheck handles (mono and stereo only).
    #[error("{} has {channels} channels; only mono and stereo are supported", path.display())]
    UnsupportedChannels {
        /// The file concerned.
        path: PathBuf,
        /// The channel count found.
        channels: usize,
    },
    /// The file could not be parsed or decoded.
    #[error("corrupt file {}: {detail}", path.display())]
    Corrupt {
        /// The file concerned.
        path: PathBuf,
        /// What failed.
        detail: String,
    },
    /// An operating-system I/O failure.
    #[error("I/O error on {}: {source}", path.display())]
    Io {
        /// The file concerned.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The job was cancelled by the user.
    #[error("cancelled")]
    Cancelled,
    /// The output SoundCheck would write is not DJ-safe (a sample rate other than 44.1 or
    /// 48 kHz, an output past the 4 GiB RIFF/AIFF limit), so it is not written.
    #[error("{} cannot be written DJ-safe: {reason}", path.display())]
    NotDjSafe {
        /// The file concerned.
        path: PathBuf,
        /// What is not DJ-safe.
        reason: String,
    },
    /// The file is DRM-protected and cannot be decoded.
    #[error("{} is DRM-protected", path.display())]
    DrmProtected {
        /// The file concerned.
        path: PathBuf,
    },
    /// The requested gain would push the true peak over the ceiling.
    #[error("gain of {needed_db:.2} dB would exceed the ceiling by {over_db:.2} dB")]
    WouldClip {
        /// Gain that would be needed to reach the target, in dB.
        needed_db: f64,
        /// By how much the ceiling would be exceeded, in dB.
        over_db: f64,
    },
    /// The beat-tracking model files were not found.
    #[error("beat-tracking model not found; looked in {}", searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))]
    ModelUnavailable {
        /// Directories tried, in order.
        searched: Vec<PathBuf>,
    },
    /// A caller passed an invalid value.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A failure inside SoundCheck or a bundled model that the user cannot fix.
    #[error("internal error: {0}")]
    Internal(String),
    /// The file lies on a rekordbox USB export (a `PIONEER` folder at the root of its volume).
    /// rekordbox keeps its own analysis of those files, so SoundCheck never writes there.
    #[error(
        "{} is on a rekordbox USB export ({}); SoundCheck does not change files there. \
         Process the files in your library, then export again from rekordbox",
        path.display(),
        volume.display()
    )]
    RekordboxUsbExport {
        /// The file or folder concerned.
        path: PathBuf,
        /// The volume's root.
        volume: PathBuf,
    },
    /// The file cannot be replaced in place; writing a copy to a folder still works.
    #[error("{} cannot be changed in place: {reason}", path.display())]
    InPlaceRefused {
        /// The file concerned.
        path: PathBuf,
        /// Why.
        reason: InPlaceRefusal,
    },
    /// A volume has less free space than the write needs.
    #[error(
        "not enough free space on {}: {needed_bytes} bytes needed, {free_bytes} free. \
         Free some space or choose another folder",
        volume.display()
    )]
    NoSpace {
        /// The volume's root.
        volume: PathBuf,
        /// Bytes the write needs, a 64 MiB margin included.
        needed_bytes: u64,
        /// Bytes free for this user.
        free_bytes: u64,
    },
    /// The file written did not read back as it was written; it was removed and the original
    /// is untouched.
    #[error("the new version of {} did not verify ({detail}); the original is untouched", path.display())]
    VerifyFailed {
        /// The file that was being processed.
        path: PathBuf,
        /// What differed.
        detail: String,
    },
    /// The file changed underneath SoundCheck (during processing, or since it was processed when
    /// undoing).
    #[error("{} changed {detail}", path.display())]
    FileChanged {
        /// The file concerned.
        path: PathBuf,
        /// What changed and what to do.
        detail: String,
    },
    /// No finished change of this file is recorded that could be undone.
    #[error("nothing to undo for {}: no change of it is recorded", path.display())]
    NothingToUndo {
        /// The file concerned.
        path: PathBuf,
    },
    /// The output path exists; SoundCheck never overwrites a file it did not back up.
    #[error("{} already exists; SoundCheck does not overwrite it", path.display())]
    AlreadyExists {
        /// The existing file.
        path: PathBuf,
    },
}

/// Why a file cannot be replaced in place. Each reason is something replacing the file would
/// break or something the system refuses; writing a copy to a folder is not affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InPlaceRefusal {
    /// The path is a symbolic link.
    Symlink,
    /// The file is locked in the Finder (the user or system immutable flag).
    FinderLocked,
    /// The file has an access control list, which a replaced file would not have.
    HasAcl,
    /// The file is read-only for this user.
    ReadOnlyFile,
    /// The file's folder is read-only or locked, so no file can be created next to it.
    ReadOnlyFolder,
    /// The file has other hard links, which replacing it would break.
    HardLinked {
        /// The file's link count.
        links: u64,
    },
}

impl std::fmt::Display for InPlaceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Symlink => f.write_str(
                "it is a symbolic link. Choose the file it points to, or write a copy to a folder",
            ),
            Self::FinderLocked => f.write_str(
                "it is locked (Finder: Get Info > Locked). Unlock it, or write a copy to a folder",
            ),
            Self::HasAcl => f.write_str(
                "it has an access control list, which the new version would lose. Remove the \
                 list, or write a copy to a folder",
            ),
            Self::ReadOnlyFile => f.write_str(
                "it is read-only. Allow writing (Finder: Get Info > Sharing & Permissions), or \
                 write a copy to a folder",
            ),
            Self::ReadOnlyFolder => f.write_str(
                "its folder is read-only or locked. Allow writing to the folder, or write a copy \
                 to another folder",
            ),
            Self::HardLinked { links } => write!(
                f,
                "it has {links} hard links, which replacing it would break. Write a copy to a \
                 folder"
            ),
        }
    }
}

/// Result alias for SoundCheck library crates.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// The IPC error class for this error.
    #[must_use]
    pub fn kind(&self) -> IpcErrorKind {
        match self {
            Self::UnsupportedFormat { .. } => IpcErrorKind::UnsupportedFormat,
            Self::UnsupportedChannels { .. } => IpcErrorKind::UnsupportedChannels,
            Self::Corrupt { .. } => IpcErrorKind::Corrupt,
            Self::Io { .. } => IpcErrorKind::Io,
            Self::Cancelled => IpcErrorKind::Cancelled,
            Self::NotDjSafe { .. } => IpcErrorKind::NotDjSafe,
            Self::DrmProtected { .. } => IpcErrorKind::DrmProtected,
            Self::WouldClip { .. } => IpcErrorKind::WouldClip,
            Self::ModelUnavailable { .. } => IpcErrorKind::ModelUnavailable,
            Self::InvalidArgument(_) => IpcErrorKind::InvalidArgument,
            Self::Internal(_) => IpcErrorKind::Internal,
            Self::RekordboxUsbExport { .. } => IpcErrorKind::RekordboxUsbExport,
            Self::InPlaceRefused { .. } => IpcErrorKind::InPlaceRefused,
            Self::NoSpace { .. } => IpcErrorKind::NoSpace,
            Self::VerifyFailed { .. } => IpcErrorKind::VerifyFailed,
            Self::FileChanged { .. } => IpcErrorKind::FileChanged,
            Self::NothingToUndo { .. } => IpcErrorKind::NothingToUndo,
            Self::AlreadyExists { .. } => IpcErrorKind::AlreadyExists,
        }
    }
}
