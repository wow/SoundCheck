//! Reading tags. Hints read with lofty, which SoundCheck uses read-only: BPM, genre, title and
//! artist, hints for the octave and meter choice and for display, never trusted over the audio
//! (a missing or unreadable tag is simply `None`), and the embedded cover. [`scan`] reads what
//! lofty does not expose with SoundCheck's own parsers: Serato data, the `SOUNDCHECK` record
//! and the loudness items a level change makes stale ([`id3_loudness_label`],
//! [`is_vorbis_loudness`]).

mod loudness;
mod scan;

use std::path::Path;

use lofty::file::TaggedFileExt;
use lofty::tag::ItemKey;
use sc_core::Bpm;
use sc_core::analysis::TagHints;

pub use loudness::{id3_loudness_label, is_vorbis_loudness};
pub use scan::{TagScan, scan};

/// Reads the hints from `path`; any read or parse problem yields the empty default.
#[must_use]
pub fn read_hints(path: &Path) -> TagHints {
    let Ok(file) = lofty::read_from_path(path) else {
        return TagHints::default();
    };
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return TagHints::default();
    };
    let bpm = tag
        .get_string(ItemKey::Bpm)
        .or_else(|| tag.get_string(ItemKey::IntegerBpm))
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|b| b.is_finite() && *b > 0.0)
        .map(Bpm);
    let text = |key: ItemKey| {
        tag.get_string(key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    TagHints {
        bpm,
        genre: text(ItemKey::Genre),
        title: text(ItemKey::TrackTitle),
        artist: text(ItemKey::TrackArtist),
    }
}

/// Largest cover the grid view shows; bigger art is left out rather than slowing the view.
pub const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;

/// An embedded picture as stored in the file: its media type and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    /// `image/jpeg`, `image/png` or `image/gif`.
    pub mime: &'static str,
    /// The picture as stored.
    pub bytes: Vec<u8>,
}

/// The front cover embedded in `path`, else its first picture: a JPEG, PNG or GIF of at most
/// [`MAX_COVER_BYTES`]. Any read problem, other format or larger picture yields `None`.
#[must_use]
pub fn cover(path: &Path) -> Option<Cover> {
    use lofty::picture::PictureType;
    let file = lofty::read_from_path(path).ok()?;
    let pictures = file.tags().iter().flat_map(lofty::tag::Tag::pictures);
    let picture = pictures
        .clone()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.clone().next())?;
    let bytes = picture.data();
    if bytes.len() > MAX_COVER_BYTES {
        return None;
    }
    let mime = match bytes {
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        _ => return None,
    };
    Some(Cover {
        mime,
        bytes: bytes.to_vec(),
    })
}

#[cfg(test)]
mod tests;
