//! The chunk walk: every top-level chunk of a RIFF, RF64 or FORM container with exact byte
//! ranges (Microsoft RIFF 1991 section 2, EBU Tech 3306 section 3, AIFF 1.3 section "Chunks").
//!
//! Sizes are untrusted. A chunk header is valid when its id is four printable ASCII characters
//! not starting with a space; the walk stops at the first invalid header (the rest of the file
//! becomes `trailing`), at the container end (`8 + declared size`) or at the end of the file,
//! whichever comes first. A chunk whose size runs past the end of the file is clamped to it and
//! sets `truncated`. A chunk whose size runs past the container end but not past the file is
//! kept whole (the container size is the field writers most often get wrong) and ends the walk.
//! An odd-length payload is followed by a pad byte when one fits inside the container; an odd
//! last chunk that ends exactly at the container end has none (`pad_missing`). When the byte
//! after an odd payload is not followed by a valid header but itself starts one, the pad is
//! taken as missing too (writers that never pad).

use std::io::{Cursor, Read, Seek};
use std::ops::Range;
use std::path::Path;

use sc_core::Result;

use super::{Container, MEMORY_PATH, Source, capped, corrupt, fourcc, unsupported};

/// One top-level chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Four-character id as stored (`b"fmt "`, `b"id3 "`, `b"SSND"`).
    pub id: [u8; 4],
    /// Byte offset of the 8-byte chunk header.
    pub header_offset: u64,
    /// Payload size in bytes as declared: the 32-bit size field, or the `ds64` size when that
    /// field is 0xFFFFFFFF in an RF64 file. May exceed `payload` when the chunk is clamped.
    pub size_declared: u64,
    /// Payload byte range in the file (after the header, without the pad byte), clamped to the
    /// end of the file.
    pub payload: Range<u64>,
    /// The pad byte's value when the declared size is odd and a pad byte is present.
    pub pad: Option<u8>,
    /// The declared size is odd but no pad byte follows (end of container or file, or the next
    /// chunk header starts right after the payload).
    pub pad_missing: bool,
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

/// One entry of the `ds64` table: the 64-bit size of a chunk other than `data` (EBU Tech 3306
/// `ChunkSize64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ds64Entry {
    /// Chunk id the size belongs to.
    pub id: [u8; 4],
    /// Payload size, bytes.
    pub size: u64,
}

/// The RF64 `ds64` chunk (EBU Tech 3306).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds64 {
    /// Size of the RF64 container after its 8-byte header, bytes (`riffSize`).
    pub riff_size: u64,
    /// Size of the `data` payload, bytes (`dataSize`).
    pub data_size: u64,
    /// Sample count of the `fact` chunk equivalent (`sampleCount`); informative only.
    pub sample_count: u64,
    /// 64-bit sizes of other chunks, as many as the chunk really holds.
    pub table: Vec<Ds64Entry>,
}

impl Ds64 {
    /// The 64-bit size `ds64` gives a chunk `id`: `dataSize` for `data`, otherwise the first
    /// table entry for `id`.
    #[must_use]
    pub fn size_of(&self, id: [u8; 4]) -> Option<u64> {
        if &id == b"data" {
            return Some(self.data_size);
        }
        self.table.iter().find(|e| e.id == id).map(|e| e.size)
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
    /// an `ID3v1` tag after the container, or garbage that did not parse as a chunk header.
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

    /// Byte offset of the container end as declared (`8 + form_size_declared`, saturating).
    #[must_use]
    pub fn container_end(&self) -> u64 {
        self.form_size_declared.saturating_add(8)
    }
}

/// Size of a chunk header: id and 32-bit size.
const HEADER_BYTES: u64 = 8;
/// Fixed part of a `ds64` payload: three 64-bit sizes and the table length.
const DS64_FIXED_BYTES: u64 = 28;
/// One `ds64` table entry: id and 64-bit size.
const DS64_ENTRY_BYTES: u64 = 12;

/// Whether `id` can start a chunk: four printable ASCII characters, the first not a space.
fn valid_id(id: &[u8]) -> bool {
    id.len() == 4 && id[0] != b' ' && id.iter().all(|b| (0x20..=0x7E).contains(b))
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn le64(b: &[u8]) -> u64 {
    let mut a = [0_u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

/// Lists the chunks of a RIFF/RF64/FORM file.
///
/// # Errors
/// [`sc_core::Error::Corrupt`] when the file is not a RIFF, RF64 or FORM file (or is shorter
/// than its 12-byte header), or is RF64 without a readable `ds64` chunk first;
/// [`sc_core::Error::UnsupportedFormat`] for a RIFF/FORM container that is not `WAVE`, `AIFF`
/// or `AIFC`, or for big-endian `RIFX`; [`sc_core::Error::Io`] when reading fails.
pub fn walk<R: Read + Seek>(reader: &mut R, path: &Path) -> Result<ChunkTable> {
    let mut src = Source::new(reader, path)?;
    let mut head = [0_u8; 12];
    if src.len < 12 {
        return Err(corrupt(path, 0, "shorter than a RIFF/RF64/FORM header"));
    }
    src.read_at(0, &mut head)?;
    let container = container_of(&head, path)?;
    let big = container.is_big_endian();
    let size32 = if big {
        u32::from_be_bytes([head[4], head[5], head[6], head[7]])
    } else {
        le32(&head[4..])
    };
    let ds64 = if container == Container::Rf64 {
        Some(read_ds64(&mut src)?)
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
    let stop = walk_chunks(&mut src, &mut table)?;
    table.trailing = (stop < src.len).then_some(stop..src.len);
    tracing::debug!(
        path = %path.display(),
        container = ?table.container,
        chunks = table.chunks.len(),
        truncated = table.truncated,
        trailing_bytes = table.trailing.as_ref().map_or(0, |t| t.end - t.start),
        "iff walk"
    );
    Ok(table)
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

/// Reads the `ds64` chunk that must follow an RF64 header.
fn read_ds64<R: Read + Seek>(src: &mut Source<'_, R>) -> Result<Ds64> {
    let at = 12;
    let path = src.path();
    let missing = || corrupt(path, at, "RF64 file without a ds64 chunk first");
    if src.len < at + HEADER_BYTES {
        return Err(missing());
    }
    let mut header = [0_u8; 8];
    src.read_at(at, &mut header)?;
    if &header[..4] != b"ds64" {
        return Err(missing());
    }
    let payload_at = at + HEADER_BYTES;
    let available = u64::from(le32(&header[4..])).min(src.len - payload_at);
    if available < DS64_FIXED_BYTES {
        return Err(corrupt(path, at, "ds64 chunk shorter than 28 bytes"));
    }
    let mut fixed = [0_u8; 28];
    src.read_at(payload_at, &mut fixed)?;
    let declared_entries = u64::from(le32(&fixed[24..]));
    let entries = declared_entries.min((available - DS64_FIXED_BYTES) / DS64_ENTRY_BYTES);
    let mut table = Vec::new();
    for i in 0..entries {
        let mut entry = [0_u8; 12];
        src.read_at(
            payload_at + DS64_FIXED_BYTES + i * DS64_ENTRY_BYTES,
            &mut entry,
        )?;
        table.push(Ds64Entry {
            id: [entry[0], entry[1], entry[2], entry[3]],
            size: le64(&entry[4..]),
        });
    }
    Ok(Ds64 {
        riff_size: le64(&fixed),
        data_size: le64(&fixed[8..]),
        sample_count: le64(&fixed[16..]),
        table,
    })
}

/// Walks from byte 12; returns the offset where the walk stopped.
fn walk_chunks<R: Read + Seek>(src: &mut Source<'_, R>, table: &mut ChunkTable) -> Result<u64> {
    let big = table.container.is_big_endian();
    let limit = table.container_end().min(src.len);
    let mut pos = 12_u64;
    while pos + HEADER_BYTES <= limit {
        let mut header = [0_u8; 8];
        src.read_at(pos, &mut header)?;
        if !valid_id(&header[..4]) {
            break;
        }
        let id = [header[0], header[1], header[2], header[3]];
        let raw = if big {
            u32::from_be_bytes([header[4], header[5], header[6], header[7]])
        } else {
            le32(&header[4..])
        };
        let size_declared = match &table.ds64 {
            Some(d) if raw == u32::MAX => d.size_of(id).unwrap_or(u64::from(raw)),
            _ => u64::from(raw),
        };
        let start = pos + HEADER_BYTES;
        let declared_end = start.saturating_add(size_declared);
        let clamped = declared_end > src.len;
        let end = declared_end.min(src.len);
        let odd = size_declared % 2 == 1;
        let pad = if odd && !clamped && end < limit {
            pad_byte(src, end, limit)?
        } else {
            None
        };
        table.chunks.push(Chunk {
            id,
            header_offset: pos,
            size_declared,
            payload: start..end,
            pad,
            pad_missing: odd && pad.is_none(),
        });
        pos = end + u64::from(pad.is_some());
        if clamped {
            table.truncated = true;
            break;
        }
    }
    Ok(pos)
}

/// The pad byte at `at` (inside the container), unless the bytes say it is missing: the byte
/// itself starts a valid chunk header while the header after it is not valid.
fn pad_byte<R: Read + Seek>(src: &mut Source<'_, R>, at: u64, limit: u64) -> Result<Option<u8>> {
    let mut peek = [0_u8; 9];
    let n = capped(limit - at, peek.len());
    src.read_at(at, &mut peek[..n])?;
    let missing = n == peek.len() && valid_id(&peek[..4]) && !valid_id(&peek[1..5]);
    Ok((!missing).then_some(peek[0]))
}

#[cfg(test)]
mod tests;
