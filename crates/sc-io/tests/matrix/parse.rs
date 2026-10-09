//! An independent reader that lists every block of a file, used to check fixtures and outputs.
//!
//! It shares no code with `sc-io` or with the fixture builders: it walks RIFF/RF64 (EBU Tech
//! 3306 `ds64`) and FORM AIFF/AIFC chunks, the `ID3v2`.3/2.4 tags inside `id3 `/`ID3 ` chunks
//! (`parse_id3.rs`), FLAC metadata blocks (RFC 9639 section 8) with an optional leading `ID3v2`
//! tag and trailing `ID3v1` tag, and Vorbis comment fields, and returns them in file order with
//! their bytes. Header details (`fmt `, `COMM`, STREAMINFO, frame walking) are in `inspect.rs`.
//!
//! Bytes after the last chunk that cannot hold another chunk header are one trailing blob up to
//! the end of the file, whether the container size counts them or not (some taggers leave a
//! stray zero byte or two inside the `FORM`); [`Parsed::stray_in_container`] says how many of
//! them the container size covers, so a check can insist on 0 for a writer's output.
//!
//! What is hashed: a chunk's payload (the size field follows from it; the pad byte is reported
//! with its value), an ID3 frame's header and body (so its flags count), an ID3 extended header,
//! a FLAC block's payload (its header carries the last-block flag, which may legitimately move),
//! a Vorbis comment field's `NAME=value` bytes, and leading or trailing tags as whole blobs.

use sha2::{Digest, Sha256};

pub use super::parse_id3::{Id3Tag, frame_text};

/// What a listed block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A top-level chunk of a RIFF/RF64 or FORM container.
    Chunk,
    /// The extended header of an `ID3v2` tag (listed right after the chunk holding the tag).
    Id3ExtHeader,
    /// A frame of an `ID3v2` tag (listed after the chunk and the extended header).
    Id3Frame,
    /// A FLAC metadata block.
    FlacBlock,
    /// The vendor string of a Vorbis comment block (listed after that block).
    VorbisVendor,
    /// One `NAME=value` field of a Vorbis comment block.
    VorbisField,
    /// All FLAC audio frames, from the first frame header to the end of the stream.
    FlacFrames,
    /// An `ID3v2` tag in front of a FLAC stream, as one blob.
    LeadingTag,
    /// Bytes after the end of the RIFF/FORM container or the FLAC stream (an `ID3v1` tag).
    Trailing,
}

impl Kind {
    /// Stable name used in the golden manifest.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Chunk => "chunk",
            Self::Id3ExtHeader => "id3-ext-header",
            Self::Id3Frame => "id3-frame",
            Self::FlacBlock => "flac-block",
            Self::VorbisVendor => "vorbis-vendor",
            Self::VorbisField => "vorbis-field",
            Self::FlacFrames => "flac-frames",
            Self::LeadingTag => "leading-tag",
            Self::Trailing => "trailing",
        }
    }

    /// Whether blocks of this kind live inside a tag (ID3 chunk or Vorbis comment block).
    #[must_use]
    pub fn is_tag_item(self) -> bool {
        matches!(
            self,
            Self::Id3ExtHeader | Self::Id3Frame | Self::VorbisVendor | Self::VorbisField
        )
    }
}

/// A block as both the builders and the parser describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Block kind.
    pub kind: Kind,
    /// Chunk id (`"fmt "`), ID3 frame label (`"TXXX:SOURCE"`, `"GEOB:Serato BeatGrid"`), FLAC
    /// block name (`"APPLICATION:riff"`, `"TYPE-100"`), Vorbis field name, `"vendor"`,
    /// `"ext-header"`, `"frames"`, `"id3v2"` or `"trailing"`.
    pub id: String,
    /// Lower-case hex SHA-256 of the hashed bytes (see the module doc).
    pub sha256: String,
    /// The pad byte after an odd-length chunk, when present.
    pub pad: Option<u8>,
}

/// Container family as found in the magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// `RIFF`/`WAVE`.
    Wave,
    /// `RF64`/`WAVE` with a `ds64` chunk.
    Rf64,
    /// `FORM`/`AIFF`.
    Aiff,
    /// `FORM`/`AIFC`.
    Aifc,
    /// `fLaC`, possibly behind an `ID3v2` tag.
    Flac,
}

/// One block found in a file.
#[derive(Debug, Clone)]
pub struct Block {
    /// Block kind.
    pub kind: Kind,
    /// Identifier as in [`Listed::id`].
    pub id: String,
    /// Byte offset of the block header (of the payload for vendor, fields and blobs).
    pub offset: usize,
    /// The hashed bytes (see the module doc).
    pub bytes: Vec<u8>,
    /// Pad byte after an odd-length chunk.
    pub pad: Option<u8>,
}

impl Block {
    /// The block as a listing entry.
    #[must_use]
    pub fn listed(&self) -> Listed {
        Listed {
            kind: self.kind,
            id: self.id.clone(),
            sha256: sha256_hex(&self.bytes),
            pad: self.pad,
        }
    }
}

/// Everything the reader found in a file.
#[derive(Debug, Clone)]
pub struct Parsed {
    /// Container family.
    pub container: Container,
    /// Blocks in file order; tag items follow the chunk or block that holds them.
    pub blocks: Vec<Block>,
    /// `ID3v2` tags found in chunks, in file order.
    pub tags: Vec<Id3Tag>,
    /// Bytes after the last chunk (fewer than a chunk header) that lie inside the declared
    /// RIFF/FORM size; they start the trailing blob. Always 0 for FLAC.
    pub stray_in_container: usize,
}

impl Parsed {
    /// The listing of every block.
    #[must_use]
    pub fn listing(&self) -> Vec<Listed> {
        self.blocks.iter().map(Block::listed).collect()
    }

    /// The first block of `kind` whose id is `id`.
    #[must_use]
    pub fn find(&self, kind: Kind, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.kind == kind && b.id == id)
    }

    /// The tag items (ID3 extended header and frames, or Vorbis vendor and fields) that follow
    /// block `index`.
    #[must_use]
    pub fn items_after(&self, index: usize) -> &[Block] {
        let rest = &self.blocks[index + 1..];
        let n = rest.iter().take_while(|b| b.kind.is_tag_item()).count();
        &rest[..n]
    }
}

/// Lower-case hex SHA-256.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Lists the blocks of a WAV, RF64, AIFF, AIFF-C or FLAC file.
///
/// # Errors
/// A description of the first structural problem (unknown magic, sizes beyond the file,
/// malformed ID3 or Vorbis data, non-zero ID3 padding).
pub fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    match bytes.get(..4) {
        Some(b"RIFF" | b"RF64") => parse_riff(bytes),
        Some(b"FORM") => parse_form(bytes),
        Some(b"fLaC") => parse_flac(bytes, 0),
        Some([b'I', b'D', b'3', _]) => {
            let size = super::parse_id3::syncsafe(slice(bytes, 6, 4)?)?;
            let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
            parse_flac(bytes, 10 + size + footer)
        }
        _ => Err("unknown magic".into()),
    }
}

/// `len` bytes of `b` from `from`, or an error naming the overrun.
///
/// # Errors
/// When the range runs past the end of `b`.
pub fn slice(b: &[u8], from: usize, len: usize) -> Result<&[u8], String> {
    from.checked_add(len)
        .and_then(|end| b.get(from..end))
        .ok_or_else(|| format!("{len} bytes at {from} run past the end ({})", b.len()))
}

fn le32(b: &[u8], at: usize) -> Result<u32, String> {
    let s = slice(b, at, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn be32(b: &[u8], at: usize) -> Result<u32, String> {
    let s = slice(b, at, 4)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn le64(b: &[u8], at: usize) -> Result<u64, String> {
    let s = slice(b, at, 8)?;
    let mut a = [0_u8; 8];
    a.copy_from_slice(s);
    Ok(u64::from_le_bytes(a))
}

/// A size field as `usize`.
///
/// # Errors
/// When it does not fit.
pub fn to_usize(v: u64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| format!("size {v} does not fit in memory"))
}

fn fourcc(b: &[u8], at: usize) -> Result<String, String> {
    Ok(String::from_utf8_lossy(slice(b, at, 4)?).into_owned())
}

fn parse_riff(b: &[u8]) -> Result<Parsed, String> {
    let rf64 = &b[..4] == b"RF64";
    if slice(b, 8, 4)? != b"WAVE" {
        return Err("RIFF form is not WAVE".into());
    }
    let mut data_size = None;
    let riff_end = if rf64 {
        if fourcc(b, 12)? != "ds64" {
            return Err("RF64 without ds64 as the first chunk".into());
        }
        data_size = Some(le64(b, 28)?);
        8 + to_usize(le64(b, 20)?)?
    } else {
        8 + to_usize(u64::from(le32(b, 4)?))?
    };
    if riff_end > b.len() {
        return Err(format!("RIFF ends at {riff_end}, file at {}", b.len()));
    }
    let mut parsed = Parsed {
        container: if rf64 {
            Container::Rf64
        } else {
            Container::Wave
        },
        blocks: Vec::new(),
        tags: Vec::new(),
        stray_in_container: 0,
    };
    let stop = walk_chunks(b, 12, riff_end, data_size, false, &mut parsed)?;
    parsed.stray_in_container = riff_end - stop;
    push_blob(b, stop, b.len(), Kind::Trailing, &mut parsed);
    Ok(parsed)
}

fn parse_form(b: &[u8]) -> Result<Parsed, String> {
    let container = match slice(b, 8, 4)? {
        b"AIFF" => Container::Aiff,
        b"AIFC" => Container::Aifc,
        _ => return Err("FORM type is neither AIFF nor AIFC".into()),
    };
    let form_end = 8 + to_usize(u64::from(be32(b, 4)?))?;
    if form_end > b.len() {
        return Err(format!("FORM ends at {form_end}, file at {}", b.len()));
    }
    let mut parsed = Parsed {
        container,
        blocks: Vec::new(),
        tags: Vec::new(),
        stray_in_container: 0,
    };
    let stop = walk_chunks(b, 12, form_end, None, true, &mut parsed)?;
    parsed.stray_in_container = form_end - stop;
    push_blob(b, stop, b.len(), Kind::Trailing, &mut parsed);
    Ok(parsed)
}

fn push_blob(b: &[u8], from: usize, to: usize, kind: Kind, parsed: &mut Parsed) {
    if from < to {
        parsed.blocks.push(Block {
            kind,
            id: if kind == Kind::LeadingTag {
                "id3v2"
            } else {
                "trailing"
            }
            .into(),
            offset: from,
            bytes: b[from..to].to_vec(),
            pad: None,
        });
    }
}

/// Walks chunks in `b[pos..end]` and returns where the last one ends (pad byte included); fewer
/// than 8 bytes may follow it. `rf64_data` replaces a `data` size of 0xFFFFFFFF.
fn walk_chunks(
    b: &[u8],
    mut pos: usize,
    end: usize,
    rf64_data: Option<u64>,
    big_endian: bool,
    parsed: &mut Parsed,
) -> Result<usize, String> {
    while pos + 8 <= end {
        let id = fourcc(b, pos)?;
        let raw = if big_endian {
            be32(b, pos + 4)?
        } else {
            le32(b, pos + 4)?
        };
        let size = match rf64_data {
            Some(real) if id == "data" && raw == u32::MAX => to_usize(real)?,
            _ => to_usize(u64::from(raw))?,
        };
        let payload_end = pos + 8 + size;
        if payload_end > end {
            return Err(format!("chunk {id:?} at {pos} runs past the container end"));
        }
        let pad = (size % 2 == 1 && payload_end < end).then(|| b[payload_end]);
        let index = parsed.blocks.len();
        let is_tag = id == "id3 " || id == "ID3 ";
        parsed.blocks.push(Block {
            kind: Kind::Chunk,
            id,
            offset: pos,
            bytes: b[pos + 8..payload_end].to_vec(),
            pad,
        });
        if is_tag {
            let (tag, items) =
                super::parse_id3::parse_id3(&b[pos + 8..payload_end], pos + 8, index)?;
            parsed.tags.push(tag);
            parsed.blocks.extend(items);
        }
        pos = payload_end + usize::from(pad.is_some());
    }
    Ok(pos)
}

fn flac_block_name(ty: u8, payload: &[u8]) -> String {
    match ty {
        0 => "STREAMINFO".into(),
        1 => "PADDING".into(),
        2 => format!(
            "APPLICATION:{}",
            String::from_utf8_lossy(payload.get(..4).unwrap_or_default())
        ),
        3 => "SEEKTABLE".into(),
        4 => "VORBIS_COMMENT".into(),
        5 => "CUESHEET".into(),
        6 => "PICTURE".into(),
        other => format!("TYPE-{other}"),
    }
}

/// A FLAC stream starting at `start` (after an optional leading `ID3v2` tag); a final 128-byte
/// block starting with `TAG` is an `ID3v1` tag.
fn parse_flac(b: &[u8], start: usize) -> Result<Parsed, String> {
    if slice(b, start, 4)? != b"fLaC" {
        return Err("no fLaC marker after the leading tag".into());
    }
    let mut parsed = Parsed {
        container: Container::Flac,
        blocks: Vec::new(),
        tags: Vec::new(),
        stray_in_container: 0,
    };
    push_blob(b, 0, start, Kind::LeadingTag, &mut parsed);
    let mut pos = start + 4;
    loop {
        let head = slice(b, pos, 4)?;
        let (last, ty) = (head[0] & 0x80 != 0, head[0] & 0x7F);
        let len = usize::from(head[1]) << 16 | usize::from(head[2]) << 8 | usize::from(head[3]);
        let payload = slice(b, pos + 4, len)?;
        parsed.blocks.push(Block {
            kind: Kind::FlacBlock,
            id: flac_block_name(ty, payload),
            offset: pos,
            bytes: payload.to_vec(),
            pad: None,
        });
        if ty == 4 {
            parse_vorbis(payload, pos + 4, &mut parsed.blocks)?;
        }
        pos += 4 + len;
        if last {
            break;
        }
    }
    let has_v1 = b.len() >= pos + 128 && b[b.len() - 128..].starts_with(b"TAG");
    let audio_end = if has_v1 { b.len() - 128 } else { b.len() };
    parsed.blocks.push(Block {
        kind: Kind::FlacFrames,
        id: "frames".into(),
        offset: pos,
        bytes: b[pos..audio_end].to_vec(),
        pad: None,
    });
    push_blob(b, audio_end, b.len(), Kind::Trailing, &mut parsed);
    Ok(parsed)
}

/// Vendor string and `NAME=value` fields of a Vorbis comment block (little-endian lengths).
fn parse_vorbis(p: &[u8], base: usize, out: &mut Vec<Block>) -> Result<(), String> {
    let vendor_len = to_usize(u64::from(le32(p, 0)?))?;
    out.push(Block {
        kind: Kind::VorbisVendor,
        id: "vendor".into(),
        offset: base + 4,
        bytes: slice(p, 4, vendor_len)?.to_vec(),
        pad: None,
    });
    let mut pos = 4 + vendor_len;
    let count = le32(p, pos)?;
    pos += 4;
    for _ in 0..count {
        let len = to_usize(u64::from(le32(p, pos)?))?;
        let field = slice(p, pos + 4, len)?;
        let name_end = field
            .iter()
            .position(|c| *c == b'=')
            .ok_or("Vorbis field without '='")?;
        out.push(Block {
            kind: Kind::VorbisField,
            id: String::from_utf8_lossy(&field[..name_end]).into_owned(),
            offset: base + pos + 4,
            bytes: field.to_vec(),
            pad: None,
        });
        pos += 4 + len;
    }
    if pos != p.len() {
        return Err("bytes after the last Vorbis field".into());
    }
    Ok(())
}

/// The value of a Vorbis field block (after the first `=`).
#[must_use]
pub fn vorbis_value(block: &Block) -> String {
    let text = String::from_utf8_lossy(&block.bytes);
    text.split_once('=')
        .map(|(_, v)| v.to_string())
        .unwrap_or_default()
}
