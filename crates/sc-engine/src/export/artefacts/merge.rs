//! One folder, one rekordbox XML and one grid report, describing that folder: a batch adds to what
//! an earlier batch wrote there rather than replacing it, since the earlier copies are still there.
//! Entries are matched by location (the report by its file), the newest wins, and an entry whose
//! file is gone is dropped. Everything is sorted by that key and the XML's `TrackID`s numbered in
//! that order, so a folder's artefacts do not depend on the order its batches ran in.

use std::collections::BTreeMap;
use std::path::Path;

use sc_io::rekordbox::{Speller, XmlTrack, location, read_tracks};
use sc_io::report::{ReportRow, read_rows};

use super::{BatchRow, RowAction};

/// The tracks an XML in `dir` should list: `tracks`, what the one there lists for other files that
/// still exist. A row of this batch that the XML no longer lists takes its old entry with it.
pub(super) fn merged_tracks(dir: &Path, rows: &[BatchRow], tracks: Vec<XmlTrack>) -> Vec<XmlTrack> {
    let existing = std::fs::read(dir.join(sc_io::rekordbox::XML_FILE_NAME))
        .ok()
        .and_then(|bytes| read_tracks(&bytes))
        .unwrap_or_default();
    let mut speller = Speller::new();
    let this_batch: Vec<String> = rows
        .iter()
        .filter_map(|r| r.track.as_ref())
        .map(|t| location(&speller.spell(&t.path)))
        .collect();
    let mut merged: BTreeMap<String, XmlTrack> = existing
        .into_iter()
        .filter(|t| t.path.is_file())
        .map(|t| (location(&t.path), t))
        .filter(|(key, _)| !this_batch.contains(key))
        .collect();
    merged.extend(tracks.into_iter().map(|t| (location(&t.path), t)));
    merged.into_values().collect()
}

/// Whether a report row from an earlier batch still describes a file: the copy in `dir` for a file
/// written there, else the file itself.
fn still_there(row: &ReportRow, dir: &Path) -> bool {
    let file = Path::new(&row.file);
    let wrote_copy = [RowAction::Written, RowAction::Exported]
        .iter()
        .any(|a| a.as_str() == row.action);
    if wrote_copy && let Some(name) = file.file_name() {
        // A copy, or a file that batch replaced in place, which sits at its own path.
        return dir.join(name).is_file() || file.is_file();
    }
    file.is_file()
}

/// The rows a report in `dir` should hold: `rows`, and what the one there says about other files
/// that still exist.
pub(super) fn merged_report(dir: &Path, rows: Vec<ReportRow>) -> Vec<ReportRow> {
    let existing = std::fs::read(dir.join(sc_io::report::REPORT_FILE_NAME))
        .ok()
        .and_then(|bytes| read_rows(&bytes))
        .unwrap_or_default();
    let mut merged: BTreeMap<String, ReportRow> = existing
        .into_iter()
        .filter(|r| still_there(r, dir))
        .map(|r| (r.file.clone(), r))
        .collect();
    merged.extend(rows.into_iter().map(|r| (r.file.clone(), r)));
    merged.into_values().collect()
}

/// Tracks the XML in `dir` lists now, for the confirm step to say they stay.
#[must_use]
pub fn tracks_listed_in(dir: &Path) -> usize {
    std::fs::read(dir.join(sc_io::rekordbox::XML_FILE_NAME))
        .ok()
        .and_then(|bytes| read_tracks(&bytes))
        .map_or(0, |tracks| {
            tracks.iter().filter(|t| t.path.is_file()).count()
        })
}
