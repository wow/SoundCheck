//! The chunk walk: every top-level chunk of a RIFF, RF64 or FORM file with exact byte ranges
//! (Microsoft RIFF 1991 section 2, EBU Tech 3306 section 3, AIFF 1.3 section "Chunks").
//!
//! Sizes are untrusted; nothing is allocated from a declared size and nothing is read past the
//! end of the file. A chunk header is valid when its id is four printable ASCII characters not
//! starting with a space. The walk goes from byte 12 until an invalid header or the end of the
//! file:
//! - A chunk whose header starts inside the declared container (`8 + form_size_declared`) and
//!   whose size runs past the end of the file is clamped to it and sets `truncated`.
//! - Past the declared container end the walk continues while headers are valid and their
//!   chunks fit in the file (writers often leave a stale container size, for example after
//!   appending an `id3 ` chunk, and streaming writers leave 0); such chunks, and one that
//!   straddles the container end, are marked `beyond_container`. The first header that is
//!   invalid or does not fit ends the walk, and the rest of the file is `trailing` (stray bytes,
//!   an `ID3v1` tag).
//! - An `ID3v1` tag (the last 128 bytes start with `TAG`) is a hard end: the walk reads no
//!   header at or after it, so the tag is `trailing` byte for byte whatever the container size
//!   says. The one exception keeps a correct file correct: when the walk to the end of the file
//!   is clean (no truncation, nothing left over) and the would-be tag lies inside a chunk's
//!   payload rather than at a chunk boundary, those bytes are audio or chunk data that happen
//!   to start with `TAG`, and the walk to the end of the file stands.
//! - After an odd payload the pad byte is decided as described in `pad.rs`: a payload that
//!   ends at the end of the walk has none; one that ends exactly at the container end has one
//!   only if the next byte is 0 (no chunk id starts with 0, and some writers leave the last pad
//!   out of the container size); otherwise a zero byte is a pad and a non-zero byte is judged
//!   by which reading leads to a consistent chain of headers.
//!
//! At most [`MAX_CHUNKS`] chunks and [`MAX_DS64_ENTRIES`] `ds64` table entries are read; more is
//! treated as a corrupt file. Pad decisions read at most [`MAX_PAD_PROBES`] headers per walk;
//! after that, pads are assumed present (the specifications' rule).

use std::io::{Cursor, Read, Seek};
use std::ops::Range;
use std::path::Path;

use sc_core::Result;

use super::{Container, MEMORY_PATH, Source, corrupt, fourcc, pad, unsupported};

pub use super::ds64::{Ds64, Ds64Entry, MAX_DS64_ENTRIES};

/// Most chunks a file may have.
pub const MAX_CHUNKS: usize = 65_536;

/// Most headers the pad decisions of one walk may read (a hostile file with many odd chunks
/// stays bounded); later odd chunks are assumed padded.
pub const MAX_PAD_PROBES: u64 = 1_000_000;

/// Length of an `ID3v1` tag, bytes.
const ID3V1_BYTES: u64 = 128;

/// One top-level chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Four-character id as stored (`b"fmt "`, `b"id3 "`, `b"SSND"`).
    pub id: [u8; 4],
    /// Byte offset of the 8-byte chunk header.
    pub header_offset: u64,
    /// The 32-bit size field as stored (0xFFFFFFFF in RF64 means "see `ds64`").
    pub size_field: u32,
    /// Payload size in bytes as declared: `size_field`, or the `ds64` size when that field is
    /// 0xFFFFFFFF in an RF64 file. May exceed `payload` when the chunk is clamped.
    pub size_declared: u64,
    /// Payload byte range in the file (after the header, without the pad byte), clamped to the
    /// end of the file.
    pub payload: Range<u64>,
    /// The pad byte's value when the declared size is odd and a pad byte is present.
    pub pad: Option<u8>,
    /// The declared size is odd but no pad byte follows.
    pub pad_missing: bool,
    /// The chunk (pad byte included) does not lie entirely inside the declared container.
    pub beyond_container: bool,
}

impl Chunk {
    /// The id as text (non-printable bytes escaped).
    #[must_use]
    pub fn id_text(&self) -> String {
        fourcc(self.id)
    }

    /// Payload length in bytes (after clamping).
    #[must_use]
    pub fn payload_len(&self) -> u64 {
        self.payload.end - self.payload.start
    }

    /// Byte offset just past the chunk, pad byte included.
    #[must_use]
    pub fn end(&self) -> u64 {
        self.payload.end + u64::from(self.pad.is_some())
    }

    /// Whether the payload was cut short by the end of the file.
    #[must_use]
    pub fn is_clamped(&self) -> bool {
        self.payload_len() < self.size_declared
    }
}

/// Every chunk of a file, in file order, and what lies around them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkTable {
    /// Container family.
    pub container: Container,
    /// The container size field, bytes after the 8-byte header (RF64: `ds64` `riffSize` when
    /// the field is 0xFFFFFFFF). `8 + form_size_declared` may disagree with `file_len`.
    pub form_size_declared: u64,
    /// File length, bytes.
    pub file_len: u64,
    /// The RF64 `ds64` chunk, decoded (also listed in `chunks`).
    pub ds64: Option<Ds64>,
    /// Chunks in file order (`ds64` first in an RF64 file).
    pub chunks: Vec<Chunk>,
    /// Bytes after the last chunk (and its pad byte) up to the end of the file: stray bytes or
    /// an `ID3v1` tag, or garbage that did not parse as a chunk header.
    pub trailing: Option<Range<u64>>,
    /// A chunk's size ran past the end of the file and was clamped.
    pub truncated: bool,
}

impl ChunkTable {
    /// The first chunk with id `id`.
    #[must_use]
    pub fn find(&self, id: &[u8; 4]) -> Option<&Chunk> {
        self.chunks.iter().find(|c| &c.id == id)
    }

    /// Index in `chunks` of the first chunk with id `id`.
    #[must_use]
    pub fn position(&self, id: &[u8; 4]) -> Option<usize> {
        self.chunks.iter().position(|c| &c.id == id)
    }

    /// Byte offset of the container end as declared (`8 + form_size_declared`, saturating).
    #[must_use]
    pub fn container_end(&self) -> u64 {
        self.form_size_declared.saturating_add(8)
    }
}

/// What a header read needs to know about the file.
pub(super) struct Layout<'t> {
    /// Sizes are big-endian.
    pub big: bool,
    /// RF64 sizes for fields holding 0xFFFFFFFF.
    pub ds64: Option<&'t Ds64>,
    /// Declared container end, bytes.
    pub container_end: u64,
    /// Where the walk ends: the end of the file or the start of an `ID3v1` tag.
    pub walk_end: u64,
}

/// A valid chunk header.
pub(super) struct Header {
    /// Chunk id.
    pub id: [u8; 4],
    /// Size field as stored.
    pub size_field: u32,
    /// Payload size after `ds64` substitution.
    pub size: u64,
}

/// Whether `id` can start a chunk: four printable ASCII characters, the first not a space.
fn valid_id(id: &[u8]) -> bool {
    id.len() == 4 && id[0] != b' ' && id.iter().all(|b| (0x20..=0x7E).contains(b))
}

/// The header at `at` (which has 8 bytes before the end of the file), or `None` when its id is
/// not valid.
pub(super) fn read_header<R: Read + Seek>(
    src: &mut Source<'_, R>,
    layout: &Layout<'_>,
    at: u64,
) -> Result<Option<Header>> {
    let mut h = [0_u8; 8];
    src.read_at(at, &mut h)?;
    if !valid_id(&h[..4]) {
        return Ok(None);
    }
    let id = [h[0], h[1], h[2], h[3]];
    let size_field = if layout.big {
        u32::from_be_bytes([h[4], h[5], h[6], h[7]])
    } else {
        u32::from_le_bytes([h[4], h[5], h[6], h[7]])
    };
    let size = match layout.ds64 {
        Some(d) if size_field == u32::MAX => d.size_of(id).unwrap_or(u64::from(size_field)),
        _ => u64::from(size_field),
    };
    Ok(Some(Header {
        id,
        size_field,
        size,
    }))
}

/// Lists the chunks of a RIFF/RF64/FORM file.
///
/// # Errors
/// [`sc_core::Error::Corrupt`] when the file is not a RIFF, RF64 or FORM file (or is shorter
/// than its 12-byte header), is RF64 without a readable `ds64` chunk first, or has more than
/// [`MAX_CHUNKS`] chunks or [`MAX_DS64_ENTRIES`] `ds64` entries;
/// [`sc_core::Error::UnsupportedFormat`] for a RIFF/FORM container that is not `WAVE`, `AIFF`
/// or `AIFC`, or for big-endian `RIFX`; [`sc_core::Error::Io`] when reading fails.
pub fn walk<R: Read + Seek>(reader: &mut R, path: &Path) -> Result<ChunkTable> {
    walk_with_budget(reader, path, MAX_PAD_PROBES).map(|(table, _)| table)
}

/// [`walk`] with a given pad-probe budget; also returns the probe headers read.
pub(super) fn walk_with_budget<R: Read + Seek>(
    reader: &mut R,
    path: &Path,
    probe_budget: u64,
) -> Result<(ChunkTable, u64)> {
    let mut src = Source::new(reader, path)?;
    let mut head = [0_u8; 12];
    if src.len < 12 {
        return Err(corrupt(path, 0, "shorter than a RIFF/RF64/FORM header"));
    }
    src.read_at(0, &mut head)?;
    let container = container_of(&head, path)?;
    let size32 = if container.is_big_endian() {
        u32::from_be_bytes([head[4], head[5], head[6], head[7]])
    } else {
        u32::from_le_bytes([head[4], head[5], head[6], head[7]])
    };
    let ds64 = if container == Container::Rf64 {
        Some(super::ds64::read(&mut src)?)
    } else {
        None
    };
    let form_size_declared = match &ds64 {
        Some(d) if size32 == u32::MAX => d.riff_size,
        _ => u64::from(size32),
    };
    let mut table = ChunkTable {
        container,
        form_size_declared,
        file_len: src.len,
        ds64,
        chunks: Vec::new(),
        trailing: None,
        truncated: false,
    };
    let len = src.len;
    let mut walked = walk_chunks(&mut src, &table, len, probe_budget)?;
    let mut probes = walked.probes;
    if let Some(tag) = id3v1_start(&mut src)?
        && !walked.stands_over(tag, src.len)
    {
        walked = walk_chunks(&mut src, &table, tag, probe_budget - probes)?;
        probes += walked.probes;
    }
    table.trailing = (walked.stop < src.len).then_some(walked.stop..src.len);
    table.chunks = walked.chunks;
    table.truncated = walked.truncated;
    tracing::debug!(
        path = %path.display(),
        container = ?table.container,
        chunks = table.chunks.len(),
        truncated = table.truncated,
        trailing_bytes = table.trailing.as_ref().map_or(0, |t| t.end - t.start),
        "iff walk"
    );
    Ok((table, probes))
}

/// [`walk`] over bytes in memory (errors name the path `<memory>`).
///
/// # Errors
/// As for [`walk`].
pub fn walk_bytes(bytes: &[u8]) -> Result<ChunkTable> {
    walk(&mut Cursor::new(bytes), Path::new(MEMORY_PATH))
}

fn container_of(head: &[u8; 12], path: &Path) -> Result<Container> {
    let (magic, form) = (&head[..4], &head[8..12]);
    match (magic, form) {
        (b"RIFF", b"WAVE") => Ok(Container::Riff),
        (b"RF64" | b"BW64", b"WAVE") => Ok(Container::Rf64),
        (b"FORM", b"AIFF") => Ok(Container::Aiff),
        (b"FORM", b"AIFC") => Ok(Container::Aifc),
        (b"RIFX", _) => Err(unsupported(path, "big-endian RIFX files".into())),
        (b"RIFF" | b"RF64" | b"BW64" | b"FORM", _) => Err(unsupported(
            path,
            format!(
                "{} form type {}",
                fourcc([magic[0], magic[1], magic[2], magic[3]]),
                fourcc([form[0], form[1], form[2], form[3]])
            ),
        )),
        _ => Err(corrupt(path, 0, "not a RIFF, RF64 or FORM file")),
    }
}

/// Start of an `ID3v1` tag: the last 128 bytes, after the 12-byte header, start with `TAG`.
fn id3v1_start<R: Read + Seek>(src: &mut Source<'_, R>) -> Result<Option<u64>> {
    if src.len < 12 + ID3V1_BYTES {
        return Ok(None);
    }
    let at = src.len - ID3V1_BYTES;
    let mut magic = [0_u8; 3];
    src.read_at(at, &mut magic)?;
    Ok((&magic == b"TAG").then_some(at))
}

/// The result of one walk over the chunks.
struct Walked {
    chunks: Vec<Chunk>,
    truncated: bool,
    /// Offset where the walk stopped.
    stop: u64,
    /// Pad-probe headers read.
    probes: u64,
}

impl Walked {
    /// Whether a walk to the end of the file (`len`) stands although the last 128 bytes look
    /// like an `ID3v1` tag starting at `tag`: it is clean and the tag lies inside a payload.
    fn stands_over(&self, tag: u64, len: u64) -> bool {
        self.stop == len
            && !self.truncated
            && self
                .chunks
                .iter()
                .any(|c| c.payload.start <= tag && tag < c.payload.end)
    }
}

/// Walks from byte 12 up to `walk_end` (the end of the file or the start of an `ID3v1` tag).
fn walk_chunks<R: Read + Seek>(
    src: &mut Source<'_, R>,
    table: &ChunkTable,
    walk_end: u64,
    probe_budget: u64,
) -> Result<Walked> {
    let container_end = table.container_end();
    let layout = Layout {
        big: table.container.is_big_endian(),
        ds64: table.ds64.as_ref(),
        container_end,
        walk_end,
    };
    let mut budget = probe_budget;
    let mut chunks = Vec::new();
    let mut truncated = false;
    let mut pos = 12_u64;
    while walk_end.saturating_sub(pos) >= 8 {
        let Some(header) = read_header(src, &layout, pos)? else {
            break;
        };
        if chunks.len() == MAX_CHUNKS {
            return Err(corrupt(
                src.path(),
                pos,
                &format!("more than {MAX_CHUNKS} chunks"),
            ));
        }
        let start = pos + 8;
        let declared_end = start.saturating_add(header.size);
        let inside = pos < container_end;
        if declared_end > walk_end && !inside {
            break;
        }
        let end = declared_end.min(walk_end);
        let odd = header.size % 2 == 1;
        let pad = if !odd || end >= walk_end {
            None
        } else if end == container_end {
            zero_byte(src, end)?
        } else {
            pad::decide(src, &layout, end, &mut budget)?
        };
        let chunk_end = end + u64::from(pad.is_some());
        chunks.push(Chunk {
            id: header.id,
            header_offset: pos,
            size_field: header.size_field,
            size_declared: header.size,
            payload: start..end,
            pad,
            pad_missing: odd && pad.is_none(),
            beyond_container: chunk_end > container_end,
        });
        pos = chunk_end;
        if declared_end > walk_end {
            truncated = true;
            break;
        }
    }
    Ok(Walked {
        chunks,
        truncated,
        stop: pos,
        probes: probe_budget - budget,
    })
}

/// `Some(0)` when the byte at `at` is 0 (a pad left out of the container size).
fn zero_byte<R: Read + Seek>(src: &mut Source<'_, R>, at: u64) -> Result<Option<u8>> {
    let mut byte = [0_u8; 1];
    src.read_at(at, &mut byte)?;
    Ok((byte[0] == 0).then_some(0))
}

#[cfg(test)]
mod tests;
