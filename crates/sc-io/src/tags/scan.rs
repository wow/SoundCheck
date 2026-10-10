//! Reading a file's own tags with SoundCheck's parsers (lofty does not expose them all): the
//! Serato data it holds, its `SOUNDCHECK` record and its loudness items.
//!
//! - WAV and AIFF: every `id3 `/`ID3 ` chunk, indexed by [`crate::id3::parse_tag`].
//! - MP3: the `ID3v2` tag at the start of the file.
//! - FLAC: every `VORBIS_COMMENT` block.
//!
//! Serato keeps its data in `GEOB` objects described `Serato <name>` (ID3) and in fields named
//! `SERATO_<NAME>` (FLAC). A tag our index refuses (ID3v2.2, tag-level unsynchronisation, a
//! malformed layout) is still searched for Serato's object descriptions as ISO-8859-1 bytes,
//! because missing Serato data would let a cut move cue points. M4A and Ogg files are not read.
//! Nothing here fails: an unreadable file or tag yields what could be read.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sc_core::export::{SoundcheckRecord, TAG_SOUNDCHECK};
use sc_core::ipc::SeratoTag;
use sc_core::plan::Codec;

use super::loudness::{id3_loudness_label, is_vorbis_loudness};
use crate::flac::{self, block_type, vorbis};
use crate::id3::{self, MAX_TAG_BYTES};
use crate::iff;

/// What [`scan`] found in a file's tags.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagScan {
    /// The kinds of Serato data, each once, in [`SeratoTag`] order.
    pub serato: Vec<SeratoTag>,
    /// The first readable `SOUNDCHECK` record (ID3 `TXXX:SOUNDCHECK`, Vorbis `SOUNDCHECK`).
    pub soundcheck: Option<SoundcheckRecord>,
    /// Every loudness item, by label as the render's stale report names them
    /// (`TXXX:REPLAYGAIN_TRACK_GAIN`, `RVA2`, `COMM:iTunNORM`, `GEOB:Serato Autotags`, Vorbis
    /// field names), in file order.
    pub loudness: Vec<String>,
}

impl TagScan {
    fn add_serato(&mut self, tag: SeratoTag) {
        if let Err(at) = self.serato.binary_search(&tag) {
            self.serato.insert(at, tag);
        }
    }

    fn add_record(&mut self, value: &str) {
        if self.soundcheck.is_some() {
            return;
        }
        match SoundcheckRecord::parse(value) {
            Ok(record) => self.soundcheck = Some(record),
            Err(e) => tracing::debug!(stage = "probe", error = %e, "SOUNDCHECK tag not read"),
        }
    }
}

/// Reads the tags of `path`, a file of `codec`.
#[must_use]
pub fn scan(path: &Path, codec: Codec) -> TagScan {
    let mut out = TagScan::default();
    let Ok(mut file) = File::open(path) else {
        return out;
    };
    match codec {
        Codec::Wav | Codec::Aiff => scan_iff(&mut file, path, &mut out),
        Codec::Mp3 => scan_leading_id3(&mut file, &mut out),
        Codec::Flac => scan_flac(&mut file, path, &mut out),
        _ => {}
    }
    out
}

/// Reads `len` bytes at `start`, when `len` is at most [`MAX_TAG_BYTES`].
fn read_at<R: Read + Seek>(src: &mut R, start: u64, len: u64) -> Option<Vec<u8>> {
    if len > MAX_TAG_BYTES {
        return None;
    }
    let mut bytes = vec![0; usize::try_from(len).ok()?];
    src.seek(SeekFrom::Start(start)).ok()?;
    src.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

fn scan_iff<R: Read + Seek>(src: &mut R, path: &Path, out: &mut TagScan) {
    let Ok(header) = iff::read_header(src, path) else {
        return;
    };
    for chunk in &header.table.chunks {
        if (&chunk.id == b"id3 " || &chunk.id == b"ID3 ")
            && let Some(tag) = read_at(src, chunk.payload.start, chunk.payload_len())
        {
            scan_id3(&tag, out);
        }
    }
}

fn scan_leading_id3<R: Read + Seek>(src: &mut R, out: &mut TagScan) {
    let mut head = [0; 10];
    if src.read_exact(&mut head).is_err() || &head[..3] != b"ID3" {
        return;
    }
    let Some(size) = id3::decode_syncsafe([head[6], head[7], head[8], head[9]]) else {
        return;
    };
    let footer = if head[3] == 4 && head[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    if let Some(tag) = read_at(src, 0, 10 + u64::from(size) + footer) {
        scan_id3(&tag, out);
    }
}

/// Serato's object descriptions as they appear in a tag (ISO-8859-1, terminated).
const SERATO_OBJECTS: [&[u8]; 9] = [
    b"Serato Markers_\0",
    b"Serato Markers2\0",
    b"Serato BeatGrid\0",
    b"Serato Autotags\0",
    b"Serato Overview\0",
    b"Serato Analysis\0",
    b"Serato Offsets_\0",
    b"Serato RelVolAd\0",
    b"Serato VidAssoc\0",
];

fn scan_id3(tag: &[u8], out: &mut TagScan) {
    let Ok(index) = id3::parse_tag(tag) else {
        for object in SERATO_OBJECTS {
            if tag.windows(object.len()).any(|w| w == object) {
                let name = String::from_utf8_lossy(&object[..object.len() - 1]);
                if let Some(kind) = SeratoTag::from_geob_description(&name) {
                    out.add_serato(kind);
                }
            }
        }
        return;
    };
    for frame in &index.frames {
        if &frame.id == b"GEOB"
            && let Some(kind) = [&frame.description, &frame.description_be]
                .into_iter()
                .flatten()
                .find_map(|d| SeratoTag::from_geob_description(d))
        {
            out.add_serato(kind);
        }
        if frame.is(b"TXXX", TAG_SOUNDCHECK)
            && let Some(value) = index.txxx_value(tag, frame)
        {
            out.add_record(&value);
        }
        if let Some(label) = id3_loudness_label(frame) {
            out.loudness.push(label);
        }
    }
}

fn scan_flac<R: Read + Seek>(src: &mut R, path: &Path, out: &mut TagScan) {
    let Ok(layout) = flac::read_layout(src, path) else {
        return;
    };
    for block in &layout.blocks {
        if block.block_type != block_type::VORBIS_COMMENT {
            continue;
        }
        let Some(payload) = read_at(src, block.payload.start, block.len()) else {
            continue;
        };
        let Ok(index) = vorbis::index(&payload) else {
            continue;
        };
        for range in &index.fields {
            let field = &payload[range.clone()];
            let Some(eq) = field.iter().position(|b| *b == b'=') else {
                continue;
            };
            let (name, value) = (&field[..eq], &field[eq + 1..]);
            let name_text = String::from_utf8_lossy(name);
            if let Some(kind) = SeratoTag::from_vorbis_name(&name_text) {
                out.add_serato(kind);
            }
            if name.eq_ignore_ascii_case(TAG_SOUNDCHECK.as_bytes()) {
                out.add_record(&String::from_utf8_lossy(value));
            }
            if is_vorbis_loudness(name) {
                out.loudness.push(name_text.into_owned());
            }
        }
    }
}
