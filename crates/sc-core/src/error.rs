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
            Self::DrmProtected { .. } => IpcErrorKind::DrmProtected,
            Self::WouldClip { .. } => IpcErrorKind::WouldClip,
            Self::ModelUnavailable { .. } => IpcErrorKind::ModelUnavailable,
            Self::InvalidArgument(_) => IpcErrorKind::InvalidArgument,
        }
    }
}
