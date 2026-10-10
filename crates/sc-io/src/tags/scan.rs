//! Reading a file's own tags with SoundCheck's parsers (lofty does not expose them all): the
//! Serato data it holds, its `SOUNDCHECK` record and its loudness items.
//!
//! - WAV and AIFF: every `id3 `/`ID3 ` chunk, indexed by [`crate::id3::parse_tag`].
//! - MP3: the `ID3v2` tag at the start of the file.
//! - FLAC: every `VORBIS_COMMENT` block.
//!
//! Serato keeps its data in `GEOB` objects described `Serato <name>` (ID3) and in fields named
//! `SERATO_<NAME>` (FLAC). Detection fails safe, because missing Serato data would let a cut in
//! place move cue points: an ID3 tag our index refuses (ID3v2.2, tag-level unsynchronisation,
//! a malformed layout) or one larger than [`MAX_TAG_BYTES`] (searched in 1 MiB windows) is still
//! searched for Serato's object descriptions as ISO-8859-1 bytes, a Vorbis comment that does
//! not index is searched for `SERATO_` names, and a file or tag that cannot be read at all sets
//! [`TagScan::serato_unknown`]. M4A and Ogg files are not read (they are never written).
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
    /// The file or one of its tags could not be read (an I/O error, a container that does not
    /// parse, a tag header without a valid size), so Serato data cannot be ruled out.
    pub serato_unknown: bool,
    /// The first readable `SOUNDCHECK` record (ID3 `TXXX:SOUNDCHECK`, Vorbis `SOUNDCHECK`).
    pub soundcheck: Option<SoundcheckRecord>,
    /// Why a `SOUNDCHECK` tag that is present could not be read (a later format version, a
    /// damaged value); the first such tag. A file with one was processed before.
    pub soundcheck_unreadable: Option<String>,
    /// Every loudness item, by label as the render's stale report names them
    /// (`TXXX:REPLAYGAIN_TRACK_GAIN`, `RVA2`, `COMM:iTunNORM`, `GEOB:Serato Autotags`, Vorbis
    /// field names), in file order. Not on the row yet: the export panel's stale-tags notice
    /// is to show it before anything is written.
    pub loudness: Vec<String>,
}

impl TagScan {
    fn add_serato(&mut self, tag: SeratoTag) {
        if let Err(at) = self.serato.binary_search(&tag) {
            self.serato.insert(at, tag);
        }
    }

    fn add_record(&mut self, value: &str) {
        match SoundcheckRecord::parse(value) {
            Ok(record) => {
                if self.soundcheck.is_none() {
                    self.soundcheck = Some(record);
                }
            }
            Err(e) => {
                tracing::debug!(stage = "probe", error = %e, "SOUNDCHECK tag not read");
                if self.soundcheck_unreadable.is_none() {
                    self.soundcheck_unreadable = Some(e.to_string());
                }
            }
        }
    }

    /// Marks Serato data as not ruled out.
    fn unknown(&mut self) {
        self.serato_unknown = true;
    }
}

/// Reads the tags of `path`, a file of `codec`.
#[must_use]
pub fn scan(path: &Path, codec: Codec) -> TagScan {
    let mut out = TagScan::default();
    if !matches!(codec, Codec::Wav | Codec::Aiff | Codec::Mp3 | Codec::Flac) {
        return out;
    }
    let Ok(mut file) = File::open(path) else {
        out.unknown();
        return out;
    };
    match codec {
        Codec::Wav | Codec::Aiff => scan_iff(&mut file, path, &mut out),
        Codec::Mp3 => scan_leading_id3(&mut file, &mut out),
        _ => scan_flac(&mut file, path, &mut out),
    }
    out
}

/// Reads `len` bytes at `start` (at most [`MAX_TAG_BYTES`], checked by the caller).
fn read_at<R: Read + Seek>(src: &mut R, start: u64, len: u64) -> Option<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(len).ok()?];
    src.seek(SeekFrom::Start(start)).ok()?;
    src.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// Scans the ID3 tag of `len` bytes at `start`: indexed when it fits [`MAX_TAG_BYTES`], else
/// searched for Serato's objects window by window.
fn scan_id3_at<R: Read + Seek>(src: &mut R, start: u64, len: u64, out: &mut TagScan) {
    if len > MAX_TAG_BYTES {
        if search_stream(src, start, len, out).is_err() {
            out.unknown();
        }
        return;
    }
    match read_at(src, start, len) {
        Some(tag) => scan_id3(&tag, out),
        None => out.unknown(),
    }
}

fn scan_iff<R: Read + Seek>(src: &mut R, path: &Path, out: &mut TagScan) {
    let Ok(header) = iff::read_header(src, path) else {
        out.unknown();
        return;
    };
    for chunk in &header.table.chunks {
        if &chunk.id == b"id3 " || &chunk.id == b"ID3 " {
            scan_id3_at(src, chunk.payload.start, chunk.payload_len(), out);
        }
    }
}

fn scan_leading_id3<R: Read + Seek>(src: &mut R, out: &mut TagScan) {
    let mut head = [0; 10];
    if src.read_exact(&mut head).is_err() || &head[..3] != b"ID3" {
        return;
    }
    let Some(size) = id3::decode_syncsafe([head[6], head[7], head[8], head[9]]) else {
        out.unknown();
        return;
    };
    let footer = if head[3] == 4 && head[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    scan_id3_at(src, 0, 10 + u64::from(size) + footer, out);
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

/// Bytes read per window when a tag is too large to index.
const WINDOW_BYTES: usize = 1 << 20;

/// Marks every Serato object description found in `bytes`, in one pass over them.
fn search_objects(bytes: &[u8], out: &mut TagScan) {
    const PREFIX: &[u8] = b"Serato ";
    let mut from = 0;
    while let Some(i) = bytes[from..].iter().position(|b| *b == b'S') {
        let at = from + i;
        from = at + 1;
        if !bytes[at..].starts_with(PREFIX) {
            continue;
        }
        if let Some(object) = SERATO_OBJECTS.iter().find(|o| bytes[at..].starts_with(o)) {
            let name = String::from_utf8_lossy(&object[..object.len() - 1]);
            if let Some(kind) = SeratoTag::from_geob_description(&name) {
                out.add_serato(kind);
            }
        }
    }
}

/// Searches `len` bytes at `start` for Serato's object descriptions in windows of
/// [`WINDOW_BYTES`], each overlapping the previous one by a description's length so none is
/// missed at a boundary.
fn search_stream<R: Read + Seek>(
    src: &mut R,
    start: u64,
    len: u64,
    out: &mut TagScan,
) -> std::io::Result<()> {
    let overlap = SERATO_OBJECTS.iter().map(|o| o.len()).max().unwrap_or(1) - 1;
    src.seek(SeekFrom::Start(start))?;
    let mut buf: Vec<u8> = Vec::with_capacity(WINDOW_BYTES + overlap);
    let mut left = len;
    while left > 0 {
        let keep = buf.len().min(overlap);
        buf.drain(..buf.len() - keep);
        let n = usize::try_from(left.min(WINDOW_BYTES as u64)).unwrap_or(WINDOW_BYTES);
        let old = buf.len();
        buf.resize(old + n, 0);
        src.read_exact(&mut buf[old..])?;
        left -= n as u64;
        search_objects(&buf, out);
    }
    Ok(())
}

fn scan_id3(tag: &[u8], out: &mut TagScan) {
    let Ok(index) = id3::parse_tag(tag) else {
        search_objects(tag, out);
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
        out.unknown();
        return;
    };
    for block in &layout.blocks {
        if block.block_type != block_type::VORBIS_COMMENT {
            continue;
        }
        // A metadata block holds at most 2^24 - 1 bytes.
        let Some(payload) = read_at(src, block.payload.start, block.len()) else {
            out.unknown();
            continue;
        };
        let Ok(index) = vorbis::index(&payload) else {
            search_vorbis_names(&payload, out);
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

/// Marks the Serato fields of a comment that does not index: every `SERATO_<NAME>=` (any
/// case) in its bytes, the name read up to `=` or 32 bytes.
fn search_vorbis_names(payload: &[u8], out: &mut TagScan) {
    const PREFIX: &[u8] = b"SERATO_";
    for at in 0..payload.len().saturating_sub(PREFIX.len() - 1) {
        if !payload[at..at + PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
            continue;
        }
        let rest = &payload[at..payload.len().min(at + 32)];
        let end = rest.iter().position(|b| *b == b'=').unwrap_or(rest.len());
        if let Some(kind) = SeratoTag::from_vorbis_name(&String::from_utf8_lossy(&rest[..end])) {
            out.add_serato(kind);
        }
    }
}
