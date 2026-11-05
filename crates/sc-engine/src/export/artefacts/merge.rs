//! One folder, one rekordbox XML and one grid report, describing that folder: a batch adds to what
//! an earlier batch wrote there rather than replacing it, since the earlier copies are still there.
//! Entries are matched by location (the report by its file), the newest wins, and an entry whose
//! file is gone is dropped. Everything is sorted by that key and the XML's `TrackID`s numbered in
//! that order, so a folder's artefacts do not depend on the order its batches ran in. A file there
//! that starts like ours but cannot be read whole is refused, never taken for an empty one: writing
//! over it would lose what it lists.

use std::collections::BTreeMap;
use std::path::Path;

use sc_core::{Error, Result};
use sc_io::rekordbox::{Speller, XmlTrack, location, read_tracks};
use sc_io::report::{ReportRow, read_rows};

use super::{BatchRow, RowAction};

/// What reading `name` in `dir` as ours found: `read`'s answer, nothing when there is no such file.
///
/// # Errors
/// [`Error::Corrupt`] when `read` cannot read it whole; [`Error::Io`] when it cannot be read at all.
fn read_ours<T: Default>(
    dir: &Path,
    name: &str,
    read: impl FnOnce(&[u8]) -> Option<T>,
) -> Result<T> {
    let path = dir.join(name);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(source) => return Err(Error::Io { path, source }),
    };
    read(&bytes).ok_or_else(|| Error::Corrupt {
        path,
        detail: "it starts like one SoundCheck wrote but cannot be read, so what it lists would \
                 be lost. Nothing was written: move it away or choose another folder"
            .to_owned(),
    })
}

/// The rekordbox XML in `dir`, read whole.
///
/// # Errors
/// As [`read_ours`].
pub(crate) fn listed_in(dir: &Path) -> Result<Vec<XmlTrack>> {
    read_ours(dir, sc_io::rekordbox::XML_FILE_NAME, read_tracks)
}

/// The grid report in `dir`, read whole.
///
/// # Errors
/// As [`read_ours`].
pub(crate) fn reported_in(dir: &Path) -> Result<Vec<ReportRow>> {
    read_ours(dir, sc_io::report::REPORT_FILE_NAME, read_rows)
}

/// The tracks an XML in `dir` should list: `tracks`, what the one there lists for other files that
/// still exist. A row of this batch that the XML no longer lists takes its old entry with it.
///
/// # Errors
/// As [`read_ours`].
pub(super) fn merged_tracks(
    dir: &Path,
    rows: &[BatchRow],
    tracks: Vec<XmlTrack>,
) -> Result<Vec<XmlTrack>> {
    let existing = listed_in(dir)?;
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
    Ok(merged.into_values().collect())
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
///
/// # Errors
/// As [`read_ours`].
pub(super) fn merged_report(dir: &Path, rows: Vec<ReportRow>) -> Result<Vec<ReportRow>> {
    let existing = reported_in(dir)?;
    let mut merged: BTreeMap<String, ReportRow> = existing
        .into_iter()
        .filter(|r| still_there(r, dir))
        .map(|r| (r.file.clone(), r))
        .collect();
    merged.extend(rows.into_iter().map(|r| (r.file.clone(), r)));
    Ok(merged.into_values().collect())
}

/// Tracks the XML in `dir` lists now, for the confirm step to say they stay.
#[must_use]
pub fn tracks_listed_in(dir: &Path) -> usize {
    listed_in(dir).map_or(0, |tracks| {
        tracks.iter().filter(|t| t.path.is_file()).count()
    })
}
