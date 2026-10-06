//! The ID3 chunk of a render with tag edits: which chunk is edited and its new payload.
//!
//! A WAV or AIFF file carries its `ID3v2` tag in a chunk with the id `id3 ` or `ID3 ` (both
//! occur in either container). With exactly one such chunk, its tag is edited (see
//! [`crate::id3`]) and the chunk keeps its id and place; with none or several, or a tag that
//! cannot be edited safely, every chunk is carried unchanged and the reason is reported. The
//! tag is read into memory up to [`MAX_TAG_BYTES`], a budget of its own beside the patched
//! chunks', so a large cover never makes a render fail.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sc_core::{Error, Result};

use crate::id3::{self, Edit, EditSummary, Edited, MAX_TAG_BYTES, NotEditable};
use crate::iff::{Chunk, ChunkTable};

/// Whether a chunk id holds an `ID3v2` tag.
pub(super) fn is_id3_chunk(id: [u8; 4]) -> bool {
    &id == b"id3 " || &id == b"ID3 "
}

/// What happens to the tag of a render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TagOutcome {
    /// No edits were requested.
    NotRequested,
    /// The chunk at this index of the chunk table is to be edited.
    Pending(usize),
    /// The tag was edited.
    Edited(EditSummary),
    /// The edits were not written, for this reason.
    NotAdded(NotEditable),
}

impl TagOutcome {
    /// The outcome before any chunk is read: which chunk holds the one tag to edit.
    pub(super) fn plan(table: &ChunkTable, edits: &[Edit]) -> Self {
        if edits.is_empty() {
            return Self::NotRequested;
        }
        let mut tags = table
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| is_id3_chunk(c.id));
        match (tags.next(), tags.count()) {
            (None, _) => Self::NotAdded(NotEditable::NoTag),
            (Some((i, _)), 0) => Self::Pending(i),
            (Some(_), more) => Self::NotAdded(NotEditable::SeveralTags { count: 1 + more }),
        }
    }
}

/// Reads the tag chunk `chunk` of `path` and applies `edits`; the inner error says why the
/// tag is carried unchanged instead.
///
/// # Errors
/// [`Error::Io`] when reading fails.
pub(super) fn edit_chunk<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    chunk: &Chunk,
    edits: &[Edit],
) -> Result<std::result::Result<Edited, NotEditable>> {
    let len = chunk.payload_len();
    if len > MAX_TAG_BYTES {
        return Ok(Err(NotEditable::TooLarge { bytes: len }));
    }
    let mut payload = vec![0; usize::try_from(len).unwrap_or(0)];
    src.seek(SeekFrom::Start(chunk.payload.start))
        .and_then(|_| src.read_exact(&mut payload))
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let edited = id3::edit_tag(&payload, edits);
    if let Err(reason) = &edited {
        tracing::info!(
            path = %path.display(),
            stage = "render-plan",
            reason = %reason,
            "tags not added"
        );
    }
    Ok(edited)
}

#[cfg(test)]
mod tests;
