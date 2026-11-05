//! What is at the path a copy to a folder would take. A copy SoundCheck wrote there may be replaced,
//! which keeps its path and so whatever a DJ app has keyed to that path, but only when its sidecar
//! proves it: the sidecar says it wrote a copy, and the file's bytes are still those it recorded.
//! Anything else there is someone's file, and is left alone.

use std::path::Path;

use sc_core::Result;

use super::fsx::{exists, hash_file, hex, io_err};
use super::journal::TxnKind;
use super::meta::FileId;
use super::sidecar::{self, sidecar_path};

/// What is at the path a copy would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyAt {
    /// Nothing.
    Nothing,
    /// A copy SoundCheck wrote, unchanged since.
    Own,
    /// Something else: no sidecar, one that does not match, or the source itself.
    Foreign,
}

/// What is at `target`, the path a copy of `source` would take.
///
/// # Errors
/// [`sc_core::Error::Io`] when `target` cannot be read to tell.
pub fn copy_at(target: &Path, source: &Path) -> Result<CopyAt> {
    if !exists(target) {
        return Ok(CopyAt::Nothing);
    }
    let meta = std::fs::symlink_metadata(target).map_err(|e| io_err(target, e))?;
    if !meta.is_file() {
        return Ok(CopyAt::Foreign);
    }
    // An export into the source's own folder must never take the source for its earlier copy.
    let same_file = |a: FileId, b: FileId| a.dev == b.dev && a.ino == b.ino;
    if let (Ok(a), Ok(b)) = (FileId::read(target), FileId::read(source))
        && same_file(a, b)
    {
        return Ok(CopyAt::Foreign);
    }
    let Ok(doc) = sidecar::read(&sidecar_path(target)) else {
        return Ok(CopyAt::Foreign);
    };
    let Some(output) = doc.output.filter(|_| doc.mode == TxnKind::ToFolder) else {
        return Ok(CopyAt::Foreign);
    };
    let (bytes, hash) = hash_file(target)?;
    Ok(if bytes == output.bytes && hex(&hash) == output.blake3 {
        CopyAt::Own
    } else {
        CopyAt::Foreign
    })
}

#[cfg(test)]
mod tests;
