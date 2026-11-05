//! What is at the path a copy to a folder would take. A copy SoundCheck wrote there of this same
//! file may be replaced, which keeps its path and so whatever a DJ app has keyed to that path, but
//! only when its sidecar proves it: the sidecar says it wrote a copy, the file's bytes are still
//! those it recorded, and it was made from this file (the same resolved path, or a moved file that
//! is byte for byte what it was made from). A copy of another track of that name is not this file's
//! to replace, and anything else there is someone's file; both are left alone.

use std::path::Path;

use sc_core::Result;

use super::fsx::{exists, hash_file, hex, io_err};
use super::journal::TxnKind;
use super::meta::FileId;
use super::preflight::real_path;
use super::sidecar::{self, FileHash, SidecarDoc, hash_of_path, sidecar_path};

/// What is at the path a copy would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyAt {
    /// Nothing.
    Nothing,
    /// A copy SoundCheck wrote of this file, unchanged since.
    Own,
    /// A copy SoundCheck wrote of another file, unchanged since.
    OtherTrack,
    /// Something else: no sidecar, one that does not match, or the source itself.
    Foreign,
}

/// What is at `target`, the path a copy of `source` would take.
///
/// # Errors
/// [`sc_core::Error::Io`] when `target` cannot be read to tell.
pub fn copy_at(target: &Path, source: &Path) -> Result<CopyAt> {
    Ok(proven_copy(target, source)?.0)
}

/// [`copy_at`], with the identity of an own copy as it was before its bytes were read.
pub(crate) fn proven_copy(target: &Path, source: &Path) -> Result<(CopyAt, Option<FileId>)> {
    let foreign = (CopyAt::Foreign, None);
    if !exists(target) {
        return Ok((CopyAt::Nothing, None));
    }
    let meta = std::fs::symlink_metadata(target).map_err(|e| io_err(target, e))?;
    if !meta.is_file() {
        return Ok(foreign);
    }
    let identity = FileId::of(&meta);
    // An export into the source's own folder must never take the source for its earlier copy,
    // however it is reached: `source` may be a link to `target`.
    if let Ok(source_meta) = std::fs::metadata(source) {
        let source_id = FileId::of(&source_meta);
        if identity.dev == source_id.dev && identity.ino == source_id.ino {
            return Ok(foreign);
        }
    }
    let Ok(doc) = sidecar::read(&sidecar_path(target)) else {
        return Ok(foreign);
    };
    let Some(output) = doc
        .output
        .as_ref()
        .filter(|_| doc.mode == TxnKind::ToFolder)
    else {
        return Ok(foreign);
    };
    let (bytes, hash) = hash_file(target)?;
    #[cfg(test)]
    tests::after_hash(target);
    // Read first and compared after: a change while it was read is not proven by what was read.
    if bytes != output.bytes || hex(&hash) != output.blake3 || FileId::read(target)? != identity {
        return Ok(foreign);
    }
    Ok(if made_from(&doc, source)? {
        (CopyAt::Own, Some(identity))
    } else {
        (CopyAt::OtherTrack, None)
    })
}

/// Whether the copy `doc` describes was made from `source`: the path it was made from, or a moved
/// file that is still byte for byte the one it was made from. A sidecar without the path's hash
/// goes by the bytes alone, which at worst refuses to replace a copy of an edited file.
fn made_from(doc: &SidecarDoc, source: &Path) -> Result<bool> {
    let resolved = real_path(source)?;
    if doc.source_path_hash.as_deref() == Some(hash_of_path(&resolved).as_str()) {
        return Ok(true);
    }
    let Some(FileHash { blake3, bytes }) = &doc.original else {
        return Ok(false);
    };
    let (source_bytes, source_hash) = hash_file(&resolved)?;
    Ok(source_bytes == *bytes && hex(&source_hash) == *blake3)
}

#[cfg(test)]
mod tests;
