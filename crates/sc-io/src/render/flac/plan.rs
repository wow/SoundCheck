//! The output's metadata blocks, decided before a byte is written: which source blocks are
//! carried by byte range, patched (CUESHEET under a trim), edited (the Vorbis comment),
//! replaced (STREAMINFO, SEEKTABLE, PADDING; their contents are written after the frames
//! where they depend on them) and every size, plus the refusals that follow from the blocks.

use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use sc_core::{Error, Result};

use super::super::layout::MAX_PATCHED_TOTAL_BYTES;
use super::super::{BlockFate, BlockId, BlockRecord};
use crate::flac::cuesheet::{self, CuesheetError};
use crate::flac::seektable::SeekShape;
use crate::flac::vorbis::{self, EditSummary, VorbisEdit};
use crate::flac::{FlacLayout, MAX_BLOCK_BYTES, MetadataBlock, STREAMINFO_BYTES, block_type};
use crate::id3::NotEditable;

/// Where an output block's payload comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Body {
    /// Copied from this byte range of the source.
    Copy(Range<u64>),
    /// These bytes (an edited comment, a patched cue sheet).
    Bytes(Vec<u8>),
    /// This many zero bytes (PADDING).
    Zeros(u32),
    /// STREAMINFO, written after the frames (34 zero bytes until then).
    StreamInfo,
    /// A SEEKTABLE of this shape, written after the frames (zero bytes until then).
    SeekTable(SeekShape),
}

/// One output metadata block (its last-block flag is set when it is written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OutBlock {
    /// Block type.
    pub block_type: u8,
    /// Payload length, bytes.
    pub len: u32,
    /// Payload source.
    pub body: Body,
}

/// What happened to the requested Vorbis comment edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TagOutcome {
    /// No edits were requested.
    NotRequested,
    /// The comment was edited.
    Edited(EditSummary),
    /// The edits were not written, for this reason (the comment, if any, is carried).
    NotAdded(NotEditable),
}

/// The planned metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    /// Blocks in output order.
    pub blocks: Vec<OutBlock>,
    /// What happened to each source block, in source order.
    pub records: Vec<BlockRecord>,
    /// What happened to the tag edits.
    pub tags: TagOutcome,
}

impl Plan {
    /// Bytes from `fLaC` to the end of the last metadata block.
    pub fn metadata_bytes(&self) -> u64 {
        4 + self
            .blocks
            .iter()
            .map(|b| 4 + u64::from(b.len))
            .sum::<u64>()
    }
}

/// What the plan needs to know of the render.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlanArgs<'a> {
    /// Frames removed from the start.
    pub trim_frames: u64,
    /// Frames written.
    pub frames_out: u64,
    /// Vorbis comment edits.
    pub edits: &'a [VorbisEdit],
}

fn read_block<R: Read + Seek>(src: &mut R, path: &Path, block: &MetadataBlock) -> Result<Vec<u8>> {
    // A metadata block holds at most 2^24 - 1 bytes.
    let mut p = vec![0; usize::try_from(block.len()).unwrap_or(0)];
    src.seek(SeekFrom::Start(block.payload.start))
        .and_then(|_| src.read_exact(&mut p))
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(p)
}

fn corrupt(path: &Path, block: &MetadataBlock, why: &str) -> Error {
    Error::Corrupt {
        path: path.to_path_buf(),
        detail: format!(
            "{} block at byte {}: {why}",
            block.name(),
            block.header_offset
        ),
    }
}

/// The index of the edited comment block and its new payload.
type EditedComment = Option<(usize, Vec<u8>)>;

/// The edited comment, or why there is none.
fn plan_tag<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    layout: &FlacLayout,
    edits: &[VorbisEdit],
) -> Result<(EditedComment, TagOutcome)> {
    if edits.is_empty() {
        return Ok((None, TagOutcome::NotRequested));
    }
    let mut comments = layout
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.block_type == block_type::VORBIS_COMMENT);
    let (i, block) = match (comments.next(), comments.count()) {
        (None, _) => return Ok((None, TagOutcome::NotAdded(NotEditable::NoVorbisComment))),
        (Some(found), 0) => found,
        (Some(_), more) => {
            let reason = NotEditable::SeveralVorbisComments { count: 1 + more };
            return Ok((None, TagOutcome::NotAdded(reason)));
        }
    };
    let payload = read_block(src, path, block)?;
    Ok(match vorbis::edit_comment(&payload, edits) {
        Ok(edited) => (Some((i, edited.bytes)), TagOutcome::Edited(edited.summary)),
        Err(reason) => {
            tracing::info!(
                path = %path.display(),
                stage = "render-plan",
                reason = %reason,
                "tags not added"
            );
            (None, TagOutcome::NotAdded(reason))
        }
    })
}

/// Plans the metadata of the output of `layout` (the file `path`, read through `src`).
///
/// # Errors
/// [`Error::Corrupt`] for a SEEKTABLE whose length is not whole seek points or a CUESHEET that
/// does not parse under a trim; [`Error::InvalidArgument`] for a trim that would break a CD-DA
/// cue sheet; [`Error::UnsupportedFormat`] when the cue sheets to patch hold more than
/// [`MAX_PATCHED_TOTAL_BYTES`]; [`Error::Io`] when reading fails.
pub(super) fn plan<R: Read + Seek>(
    src: &mut R,
    path: &Path,
    layout: &FlacLayout,
    args: PlanArgs<'_>,
) -> Result<Plan> {
    let (edited, tags) = plan_tag(src, path, layout, args.edits)?;
    let mut growth: i64 = 0;
    if let Some((i, bytes)) = &edited {
        growth = i64::try_from(bytes.len()).unwrap_or(i64::MAX)
            - i64::try_from(layout.blocks[*i].len()).unwrap_or(0);
    }
    let mut edited = edited;
    let mut held = 0_u64;
    let mut blocks = Vec::with_capacity(layout.blocks.len());
    let mut records = Vec::with_capacity(layout.blocks.len());
    for (i, block) in layout.blocks.iter().enumerate() {
        let carried = || (BlockFate::Carried, Body::Copy(block.payload.clone()));
        let (fate, body) = match block.block_type {
            block_type::STREAMINFO => (BlockFate::Replaced, Body::StreamInfo),
            block_type::SEEKTABLE => {
                let payload = read_block(src, path, block)?;
                let shape = SeekShape::of(&payload).map_err(|why| corrupt(path, block, why))?;
                (BlockFate::Replaced, Body::SeekTable(shape))
            }
            block_type::VORBIS_COMMENT => match edited.take_if(|(at, _)| *at == i) {
                Some((_, bytes)) => (BlockFate::Edited, Body::Bytes(bytes)),
                None => carried(),
            },
            block_type::CUESHEET if args.trim_frames > 0 => {
                held += block.len();
                if held > MAX_PATCHED_TOTAL_BYTES {
                    return Err(Error::UnsupportedFormat {
                        path: path.to_path_buf(),
                        detail: format!(
                            "the cue sheets to rewrite hold more than {MAX_PATCHED_TOTAL_BYTES} \
                             bytes"
                        ),
                    });
                }
                let mut payload = read_block(src, path, block)?;
                match cuesheet::shift_cuesheet(&mut payload, args.trim_frames, args.frames_out) {
                    Ok(true) => (BlockFate::Patched, Body::Bytes(payload)),
                    Ok(false) => carried(),
                    Err(CuesheetError::Malformed(why)) => return Err(corrupt(path, block, why)),
                    Err(CuesheetError::CdDaAlignment) => {
                        return Err(Error::InvalidArgument(format!(
                            "{}: a head trim of {} samples is not a whole number of CD sectors \
                             ({} samples), which the file's CD-DA cue sheet requires",
                            path.display(),
                            args.trim_frames,
                            cuesheet::CD_SECTOR_SAMPLES
                        )));
                    }
                }
            }
            block_type::PADDING => {
                // The first PADDING block absorbs the comment's change in size, so the metadata
                // keeps its length when it can.
                let len = i64::try_from(block.len()).unwrap_or(0);
                let new = (len - growth).clamp(0, i64::from(MAX_BLOCK_BYTES));
                growth -= len - new;
                (
                    BlockFate::Replaced,
                    Body::Zeros(u32::try_from(new).unwrap_or(0)),
                )
            }
            _ => carried(),
        };
        let len = match &body {
            Body::Copy(r) => r.end - r.start,
            Body::Bytes(b) => b.len() as u64,
            Body::Zeros(n) => u64::from(*n),
            Body::StreamInfo => STREAMINFO_BYTES as u64,
            Body::SeekTable(shape) => shape.bytes() as u64,
        };
        records.push(BlockRecord {
            id: BlockId::FlacBlock(block.block_type),
            fate,
            source_bytes: block.len(),
        });
        blocks.push(OutBlock {
            block_type: block.block_type,
            // Every payload is at most a source block's length or an edited comment that
            // `edit_comment` checked against the same 24-bit limit.
            len: u32::try_from(len).unwrap_or(MAX_BLOCK_BYTES),
            body,
        });
    }
    Ok(Plan {
        blocks,
        records,
        tags,
    })
}

#[cfg(test)]
mod tests;
