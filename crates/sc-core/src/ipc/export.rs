//! What an export sends and receives over IPC: the request, the row each file shows (at most
//! [`super::MAX_ROW_BYTES`] of JSON whatever the file holds, which [`ExportRow`]'s bounded lists
//! see to), the batch's artefacts, and the answers of `plan_export`, `revert_file`, `write_rekordbox_xml` and
//! `forget_pending`. The written file's analysis, with its evidence, never crosses: the row
//! carries the numbers the table shows.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::recovery::RecoveryStatus;
use super::track::RowUpdate;
use super::{FileEntry, IpcError, JobStage};
use crate::export::{Cut, ExportNotice, ExportSettings, ExportSkip, GridCheck, XmlOnlyReason};

/// Most JSON bytes the notes of one [`ExportRow`] take; [`bounded_strings`] keeps them there.
pub const MAX_ROW_NOTES_JSON_BYTES: usize = 700;

/// Most JSON bytes the stale tag labels of one [`ExportVerification`] take.
pub const MAX_STALE_TAGS_JSON_BYTES: usize = 300;

/// Export these files with these settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    /// Files by id, in the order to export them; each once.
    pub file_ids: Vec<u32>,
    /// The settings.
    pub settings: ExportSettings,
    /// The absolute folder copies go to: required by, and only by, a `folder` place.
    pub out_dir: Option<String>,
}

/// What exporting would do with one file, before anything is written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreviewRow {
    /// The file.
    pub file_id: u32,
    /// What its row would show: `stage` `processing` for a file that would be written, `skipped`
    /// for one that would not, with the plan's notes worded. `None` when it could not be planned.
    pub row: Option<Box<ExportRow>>,
    /// Why it could not be planned: not analysed yet, or its start could not be read.
    pub error: Option<IpcError>,
}

/// What `plan_export` found: the confirm step's numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    /// One row per file named, in that order.
    pub rows: Vec<ExportPreviewRow>,
    /// Where the originals go, for an export in place.
    pub backup_folder: Option<String>,
    /// Where the rekordbox XML and grid report go, with the XML on: the export's folder, or for
    /// an export in place the folder a new dated folder is made in.
    pub artefacts_folder: Option<String>,
}

/// What exporting does with a file, as the row's Action column shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportAction {
    /// The file is written: this gain, and this at its start (its cut, or why nothing is cut;
    /// `gridOnly` changes no audio).
    Write {
        /// Gain applied to every sample, dB; 0 for grid only.
        gain_db: f64,
        /// What happens at the start of the file.
        cut: Cut,
    },
    /// The file is left as it is; only the batch's rekordbox XML carries its grid.
    XmlOnly {
        /// Why.
        reason: XmlOnlyReason,
    },
    /// The file is left out.
    Skip {
        /// Why.
        reason: ExportSkip,
    },
}

/// What was checked of a written file before it replaced anything: the row's "Verified" chip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportVerification {
    /// Every chunk or metadata block of the source.
    pub blocks: u32,
    /// Those copied byte for byte.
    pub carried: u32,
    /// Those copied with positions or loudness fields rewritten.
    pub patched: u32,
    /// Those holding a tag with SoundCheck's items written, every other item copied.
    pub edited: u32,
    /// Those written anew (format and audio chunks; FLAC `STREAMINFO`, `SEEKTABLE`, `PADDING`).
    pub replaced: u32,
    /// Those left out.
    pub dropped: u32,
    /// Tag items SoundCheck added (appended, or replaced in place).
    pub tags_added: u32,
    /// Frames in the source.
    #[ts(type = "number")]
    pub frames_in: u64,
    /// Frames written.
    #[ts(type = "number")]
    pub frames_out: u64,
    /// Output bits per sample.
    pub bits_out: u16,
    /// The samples are an exact copy or shift of the source's.
    pub exact: bool,
    /// TPDF dither was added.
    pub dithered: bool,
    /// Loudness tags the gain left stale, as labelled in the file (carried unchanged; bounded to
    /// [`MAX_STALE_TAGS_JSON_BYTES`]).
    pub stale_loudness_tags: Vec<String>,
}

/// Where one file of an export stands, and all its row shows: sent whole with every step, and at
/// most [`super::MAX_ROW_BYTES`] of JSON.
// Four independent facts the row shows, not states of one thing.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportRow {
    /// `processing` (planned, being written), `writing` (in place and verified, being checked),
    /// `done` or `skipped`.
    pub stage: JobStage,
    /// What exporting does with the file.
    pub action: ExportAction,
    /// Positions in the written file moved: its start was cut (the row's badge).
    pub positions_shift: bool,
    /// The file has a grid, but it needs review and was not confirmed, so its tempo and bar 1
    /// are not written.
    pub grid_withheld: bool,
    /// What the user should know about the written file.
    pub notices: Vec<ExportNotice>,
    /// What was verified, once written.
    pub verification: Option<ExportVerification>,
    /// The exported grid checked against the written file's own analysis, once done.
    pub grid_check: Option<GridCheck>,
    /// The user's grid edit was carried over to the output, once done.
    pub edit_carried: bool,
    /// The file was changed in place, with a backup, so `revert_file` can put it back. A copy in
    /// a folder leaves the original as it was: nothing to revert.
    pub revertible: bool,
    /// What did not work after the write (the file stays written) and what the render could not
    /// do, in order; bounded to [`MAX_ROW_NOTES_JSON_BYTES`], so the grid report holds the rest.
    pub notes: Vec<String>,
}

/// Where a batch's rekordbox XML and grid report are: the footer's links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ExportArtefacts {
    /// The folder holding them, for the Finder link.
    pub folder: String,
    /// The rekordbox XML.
    pub xml: Option<String>,
    /// The grid report; `None` for an XML written alone to a file the user named.
    pub report: Option<String>,
    /// Tracks the XML lists.
    pub listed: u32,
}

/// What `revert_file` did: the original is back, and the row with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RevertResult {
    /// The file as its headers now say.
    pub entry: FileEntry,
    /// The row's analysis and plan: the original's measurements and its saved edit. `None` when
    /// the original could not be analysed again (`notes` say why; analyse the row again).
    pub update: Option<Box<RowUpdate>>,
    /// The backup it was restored from, which is kept.
    pub backup: String,
    /// Earlier changes of the file still in effect; another revert goes one further back.
    pub earlier_changes: u32,
    /// What could not be restored, or analysed afterwards.
    pub notes: Vec<String>,
}

/// Write a rekordbox XML of these files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WriteXmlRequest {
    /// Files by id, in the order to list them; each once.
    pub file_ids: Vec<u32>,
    /// An absolute file ending in `.xml` to write the XML to (one SoundCheck wrote there before
    /// is replaced; any other is refused). `None` writes the XML and the grid report into a new
    /// folder under the exports folder.
    pub out: Option<String>,
}

/// A file the XML does not list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct XmlNotListed {
    /// The file.
    pub file_id: u32,
    /// Why.
    pub why: String,
}

/// What `write_rekordbox_xml` wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct XmlWritten {
    /// Where it is.
    pub artefacts: ExportArtefacts,
    /// The files it does not list, in the order given.
    pub not_listed: Vec<XmlNotListed>,
}

/// What `forget_pending` did: nothing was deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ForgetResult {
    /// The file the change concerned.
    pub path: String,
    /// The original's backup, when it may still exist.
    pub backup: Option<String>,
    /// Every file the change made that may still exist; none was deleted.
    pub kept: Vec<String>,
    /// The recovery report without the forgotten change.
    pub status: RecoveryStatus,
}

/// The bytes `c` takes inside a JSON string.
fn json_char_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
        c if u32::from(c) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `items` in order, cut so that they take at most `max_json_bytes` as a JSON array of strings:
/// the item that crosses it ends in `...`, and those after it are dropped. They are read by a
/// person; whoever needs them whole reads the log or the grid report.
#[must_use]
pub fn bounded_strings(
    items: impl IntoIterator<Item = String>,
    max_json_bytes: usize,
) -> Vec<String> {
    const MARK: &str = "...";
    // Quotes and a comma per item, and the array's brackets.
    const OVERHEAD: usize = 3;
    let mut left = max_json_bytes.saturating_sub(2);
    let mut out = Vec::new();
    for item in items {
        let Some(room) = left.checked_sub(OVERHEAD) else {
            break;
        };
        let whole: usize = item.chars().map(json_char_len).sum();
        if whole <= room {
            left = room - whole;
            out.push(item);
            continue;
        }
        let Some(room) = room.checked_sub(MARK.len()) else {
            break;
        };
        let mut used = 0;
        let cut: String = item
            .chars()
            .take_while(|&c| {
                used += json_char_len(c);
                used <= room
            })
            .collect();
        out.push(cut + MARK);
        break;
    }
    out
}
