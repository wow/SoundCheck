//! Reading what the export planner needs from a source file: codec, depth, floating point,
//! whether it holds Serato data (found by [`sc_io::probe`]) and whether it holds the one tag
//! SoundCheck edits. The source hash is not read here (it stays unset).

use std::fs::File;
use std::path::Path;

use sc_core::ipc::FileInfo;
use sc_core::plan::Codec;
use sc_io::iff::{self, ChunkTable};

use super::{ExportSource, SeratoPresence};

impl ExportSource {
    /// What the headers and tags in `info` say, Serato data included; `has_tag` stays unset.
    #[must_use]
    pub fn from_info(info: &FileInfo) -> Self {
        Self {
            codec: info.codec,
            bits_per_sample: info.bits_per_sample,
            float: info.float,
            serato: if info.serato {
                SeratoPresence::Present
            } else if info.serato_unknown {
                SeratoPresence::Unknown
            } else {
                SeratoPresence::Absent
            },
            ..Self::default()
        }
    }

    /// Reads `path`: its headers ([`sc_io::probe`]) and, for WAV and AIFF, the chunk table, whose
    /// sample encoding and valid bits replace the probe's and whose ID3 chunks say whether there
    /// is a tag to edit. A file whose chunks cannot be read keeps the probe's answers.
    #[must_use]
    pub fn read(path: &Path) -> Self {
        let mut source = Self::from_info(&sc_io::probe(path));
        if matches!(source.codec, Codec::Wav | Codec::Aiff) {
            let header = File::open(path)
                .ok()
                .and_then(|mut f| iff::read_header(&mut f, path).ok());
            if let Some(header) = header {
                source.float = header.format.encoding.is_float();
                source.bits_per_sample = u8::try_from(header.format.valid_bits).ok();
                source.has_tag = one_id3_chunk(&header.table);
            }
        }
        source
    }
}

/// Whether the table holds exactly one ID3 chunk (`id3 ` or `ID3 `): the tag the writer edits.
fn one_id3_chunk(table: &ChunkTable) -> bool {
    table
        .chunks
        .iter()
        .filter(|c| &c.id == b"id3 " || &c.id == b"ID3 ")
        .count()
        == 1
}

#[cfg(test)]
mod tests;
