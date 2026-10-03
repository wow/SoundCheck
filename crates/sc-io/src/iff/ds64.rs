//! The RF64 `ds64` chunk (EBU Tech 3306 section 3.2): 64-bit sizes of the container, the
//! `data` chunk and any other chunk whose 32-bit size field holds 0xFFFFFFFF.

use std::io::{Read, Seek};

use sc_core::Result;

use super::{Source, corrupt};

/// Most `ds64` table entries a file may have.
pub const MAX_DS64_ENTRIES: u64 = 65_536;

/// Offset of the `ds64` header: it must be the first chunk.
const AT: u64 = 12;
/// Fixed part of a `ds64` payload: three 64-bit sizes and the table length.
const FIXED_BYTES: u64 = 28;
/// One table entry: id and 64-bit size (`ChunkSize64`).
const ENTRY_BYTES: u64 = 12;

/// One entry of the `ds64` table: the 64-bit size of a chunk other than `data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ds64Entry {
    /// Chunk id the size belongs to.
    pub id: [u8; 4],
    /// Payload size, bytes.
    pub size: u64,
}

/// The RF64 `ds64` chunk.
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

fn le64(b: &[u8]) -> u64 {
    let mut a = [0_u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

/// Reads the `ds64` chunk that must follow an RF64 header. The table holds as many entries as
/// both the declared count and the chunk's bytes (clamped to the file) allow.
pub(super) fn read<R: Read + Seek>(src: &mut Source<'_, R>) -> Result<Ds64> {
    let path = src.path();
    let missing = || corrupt(path, AT, "RF64 file without a ds64 chunk first");
    if src.len < AT + 8 {
        return Err(missing());
    }
    let mut header = [0_u8; 8];
    src.read_at(AT, &mut header)?;
    if &header[..4] != b"ds64" {
        return Err(missing());
    }
    let payload_at = AT + 8;
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let available = u64::from(size).min(src.len - payload_at);
    if available < FIXED_BYTES {
        return Err(corrupt(path, AT, "ds64 chunk shorter than 28 bytes"));
    }
    let mut fixed = [0_u8; 28];
    src.read_at(payload_at, &mut fixed)?;
    let declared = u64::from(u32::from_le_bytes([
        fixed[24], fixed[25], fixed[26], fixed[27],
    ]));
    let entries = declared.min((available - FIXED_BYTES) / ENTRY_BYTES);
    if entries > MAX_DS64_ENTRIES {
        return Err(corrupt(
            path,
            AT,
            &format!("more than {MAX_DS64_ENTRIES} ds64 table entries"),
        ));
    }
    let mut table = Vec::new();
    for i in 0..entries {
        let mut entry = [0_u8; 12];
        src.read_at(payload_at + FIXED_BYTES + i * ENTRY_BYTES, &mut entry)?;
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
