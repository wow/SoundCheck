//! Tag hints read with lofty, which SoundCheck uses read-only: BPM, genre, title and artist.
//! They are hints for the octave and meter choice and for display, never trusted over the audio;
//! a missing or unreadable tag is simply `None`.

use std::path::Path;

use lofty::file::TaggedFileExt;
use lofty::tag::ItemKey;
use sc_core::Bpm;
use sc_core::analysis::TagHints;

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
