//! The grid a batch's rekordbox XML and grid report carry for one file, and the tags it names
//! the track by.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::ExportedGrid;

/// The grid a batch's rekordbox XML and grid report carry for one file, in that file's samples:
/// the grid as exported (after the cut) for a written file, as analysed for a file left as it
/// is. A grid that needs review and was not confirmed is withheld, as from the tags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum XmlGrid {
    /// A grid that may be written.
    Grid {
        /// The grid, in the file's samples.
        grid: ExportedGrid,
        /// The file's sample rate, Hz.
        sample_rate: u32,
    },
    /// The grid needs review and the user has not confirmed it: neither its tempo nor its
    /// bar 1 is written.
    NeedsReview,
    /// No grid: loudness only, or no beats found.
    Absent,
}

impl XmlGrid {
    /// The grid, when it may be written.
    #[must_use]
    pub fn grid(&self) -> Option<(&ExportedGrid, u32)> {
        match self {
            Self::Grid { grid, sample_rate } => Some((grid, *sample_rate)),
            Self::NeedsReview | Self::Absent => None,
        }
    }
}

/// What a batch's rekordbox XML carries for one file: its grid and the tags that name it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct XmlTrackInfo {
    /// The grid, in the file's samples.
    pub grid: XmlGrid,
    /// The file's title tag.
    pub title: Option<String>,
    /// The file's artist tag.
    pub artist: Option<String>,
}
