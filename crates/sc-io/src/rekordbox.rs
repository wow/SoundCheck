//! The batch's rekordbox XML (`soundcheck-rekordbox.xml`) in the format Pioneer DJ documents as
//! "XML file format for playlists sharing" (rekordbox `xml_format_list`): `DJ_PLAYLISTS`
//! version 1.0.0 with a `PRODUCT`, a `COLLECTION` of `TRACK`s and a `PLAYLISTS` tree whose root
//! holds one playlist of every track, named per batch (`SoundCheck 2026-10-11 14.05.33`: rekordbox
//! replaces a playlist of the same name on import), its tracks keyed by `TrackID` (`KeyType` 0,
//! as rekordbox's own exports key them).
//! The format calls the tree essential and the collection's tracks outside any playlist
//! unnecessary.
//!
//! - **`TRACK`** has `TrackID` (1, 2, ... in the order listed: the XML's own key, which the
//!   playlist refers to; rekordbox matches an existing track by its location and keeps its own
//!   id) and otherwise only what SoundCheck means to set, since importing a track from the XML
//!   into rekordbox's collection overwrites the track's information with the XML's: `Name` and
//!   `Artist` from the file's own tags (the title, else the file name without its extension; no
//!   `Artist` without an artist tag), `TotalTime` (whole seconds, as the format asks),
//!   `AverageBpm` (two decimals) and `Location` ([`location`]). Nothing else: no album, genre,
//!   rating or comments.
//! - **`TEMPO`** (one per track: a static grid): `Inizio` is the first beat at or after the start
//!   of the file, in seconds with three decimals (rekordbox keeps beat positions in whole
//!   milliseconds), `Bpm` the written tempo (two decimals, [`Bpm::written`]), `Metro` `4/4`, and
//!   `Battito` that beat's number in its bar (1 to 4), counted back from bar 1 by whole beats
//!   at the written tempo. Every track listed has one: a track without a grid is not listed,
//!   since importing it could only change its information or clear rekordbox's own grid. Only
//!   4/4 grids get one ([`Tempo::of_bar1`]); how rekordbox reads `Metro` for other meters is not
//!   established.
//! - **Replacing**: an existing file is replaced only when it starts like an XML SoundCheck
//!   wrote ([`is_soundcheck_xml`]).
//! - **Bytes**: UTF-8 without a byte-order mark, LF line ends, two-space indent, attributes in a
//!   fixed order, numbers with a dot and a fixed number of decimals whatever the locale, the
//!   five XML entities escaped and characters XML 1.0 does not allow replaced by U+FFFD. The
//!   same tracks give the same bytes.

use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, Event};
use sc_core::analysis::Meter;
use sc_core::export::XmlGrid;
use sc_core::{Bpm, SampleIndex, Seconds};

mod location;

pub use location::{Speller, decode_location, location};

/// File name of a batch's rekordbox XML.
pub const XML_FILE_NAME: &str = "soundcheck-rekordbox.xml";

/// What every playlist name starts with.
pub const PLAYLIST_PREFIX: &str = "SoundCheck";

/// One track of the XML. Only tracks with a grid are listed: importing a track without a
/// `TEMPO` could only change its information, or clear the grid rekordbox has.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlTrack {
    /// The file, absolute, spelled as its folders list its names ([`Speller`]): rekordbox
    /// matches a collection entry by its location.
    pub path: PathBuf,
    /// The title tag; `None` writes the file name without its extension.
    pub title: Option<String>,
    /// The artist tag; `None` writes no `Artist`.
    pub artist: Option<String>,
    /// Playing time of the file.
    pub duration: Seconds,
    /// Its beat grid.
    pub tempo: Tempo,
}

/// A static 4/4 grid as one `TEMPO` element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    /// The first beat at or after the start of the file.
    pub inizio: Seconds,
    /// The tempo, two decimals.
    pub bpm: Bpm,
    /// The number of that beat in its bar, 1 to 4.
    pub battito: u8,
}

/// Why a track gets no `TEMPO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TempoWithheld {
    /// The file has no grid.
    Absent,
    /// The grid needs review and the user has not confirmed it (as in the tags, its tempo and
    /// bar 1 are not written).
    NeedsReview,
    /// The meter is not 4/4 (written as the badge shows it, `7/8 · 2+2+3`).
    Meter(String),
    /// The tempo does not round to a positive number of beats per minute, or the sample rate
    /// is zero.
    InvalidTempo,
}

impl Tempo {
    /// The `TEMPO` of `grid` ([`Self::of_bar1`] at its bar 1 in the file's samples, at its
    /// written tempo).
    ///
    /// # Errors
    /// [`TempoWithheld`]: no grid, a grid that needs review, a meter other than 4/4, a tempo
    /// that gives no beat length.
    pub fn of(grid: &XmlGrid) -> Result<Self, TempoWithheld> {
        match grid {
            XmlGrid::Grid { grid, sample_rate } => {
                Self::of_bar1(grid.bar1, grid.bpm, &grid.meter, *sample_rate)
            }
            XmlGrid::NeedsReview => Err(TempoWithheld::NeedsReview),
            XmlGrid::Absent => Err(TempoWithheld::Absent),
        }
    }

    /// The `TEMPO` of a grid whose bar 1 is at `bar1` samples (at `sample_rate`) with the tempo
    /// `bpm` (rounded to two decimals here) in `meter`: `Inizio` is the first beat at or after
    /// the start, `bar1 - k * 60 * sample_rate / bpm` for the largest whole `k` that keeps it
    /// at or after sample 0, and `Battito` is its number in the bar (`k` beats before a bar
    /// line is beat `1 + (-k mod 4)`).
    ///
    /// # Errors
    /// [`TempoWithheld`]: a meter other than 4/4, or a tempo or rate that gives no beat length.
    pub fn of_bar1(
        bar1: SampleIndex,
        bpm: Bpm,
        meter: &Meter,
        sample_rate: u32,
    ) -> Result<Self, TempoWithheld> {
        if *meter != Meter::four_four() {
            return Err(TempoWithheld::Meter(meter.to_string()));
        }
        let bpm = bpm.written();
        let beat = 60.0 * f64::from(sample_rate) / bpm.0;
        if !(beat.is_finite() && beat > 0.0) {
            return Err(TempoWithheld::InvalidTempo);
        }
        // u64 -> f64 is exact below 2^53 samples (about 6,000 years at 48 kHz).
        #[allow(clippy::cast_precision_loss)]
        let bar1 = bar1.0 as f64;
        let beats_back = (bar1 / beat).floor();
        let first = (bar1 - beats_back * beat).max(0.0);
        // `beats_back` is a whole number >= 0 below 2^53; mod 4 is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let back_in_bar = (beats_back % 4.0) as u8;
        Ok(Self {
            inizio: Seconds(first / f64::from(sample_rate)),
            bpm,
            battito: 1 + (4 - back_in_bar) % 4,
        })
    }
}

/// The XML of `tracks`, listed in the order given, with `product_version` (the app's version)
/// in `PRODUCT` and their playlist named `playlist` ([`playlist_name`]).
#[must_use]
pub fn xml_bytes(tracks: &[XmlTrack], product_version: &str, playlist: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(512 + 500 * tracks.len());
    // Writing into a Vec cannot fail.
    let written = write_xml(tracks, product_version, playlist, &mut out);
    debug_assert!(written.is_ok(), "writing into memory failed: {written:?}");
    out
}

/// Writes the XML of `tracks` (see [`xml_bytes`]) to `out`.
///
/// # Errors
/// Whatever `out` returns.
pub fn write_xml(
    tracks: &[XmlTrack],
    product_version: &str,
    playlist: &str,
    out: &mut impl Write,
) -> std::io::Result<()> {
    let mut w = Writer::new_with_indent(out, b' ', 2);
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    w.create_element("DJ_PLAYLISTS")
        .with_attribute(("Version", "1.0.0"))
        .write_inner_content(|w| {
            w.create_element("PRODUCT")
                .with_attributes([
                    ("Name", "SoundCheck"),
                    ("Version", &*text(product_version)),
                    ("Company", "SoundCheck"),
                ])
                .write_empty()?;
            let entries = tracks.len().to_string();
            let collection = w
                .create_element("COLLECTION")
                .with_attribute(("Entries", entries.as_str()));
            if tracks.is_empty() {
                collection.write_empty()?;
            } else {
                collection.write_inner_content(|w| {
                    for (i, track) in tracks.iter().enumerate() {
                        write_track(w, i + 1, track)?;
                    }
                    Ok(())
                })?;
            }
            write_playlists(w, tracks, &text(playlist))
        })?;
    w.get_mut().write_all(b"\n")
}

/// One `TRACK` of the collection.
fn write_track<W: Write>(w: &mut Writer<W>, id: usize, track: &XmlTrack) -> std::io::Result<()> {
    let id = id.to_string();
    let stem = track
        .path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let name = text(track.title.as_deref().unwrap_or(&stem)).into_owned();
    let artist = track.artist.as_deref().map(|a| text(a).into_owned());
    let total = whole_seconds(track.duration).to_string();
    let tempo = track.tempo;
    let bpm = format!("{:.2}", tempo.bpm.0);
    let location = location(&track.path);
    w.create_element("TRACK")
        .with_attribute(("TrackID", id.as_str()))
        .with_attribute(("Name", name.as_str()))
        .with_attributes(artist.as_deref().map(|a| ("Artist", a)))
        .with_attributes([
            ("TotalTime", total.as_str()),
            ("AverageBpm", bpm.as_str()),
            ("Location", location.as_str()),
        ])
        .write_inner_content(|w| {
            let inizio = format!("{:.3}", tempo.inizio.0);
            let battito = tempo.battito.to_string();
            w.create_element("TEMPO")
                .with_attributes([
                    ("Inizio", inizio.as_str()),
                    ("Bpm", bpm.as_str()),
                    ("Metro", "4/4"),
                    ("Battito", battito.as_str()),
                ])
                .write_empty()?;
            Ok(())
        })?;
    Ok(())
}

/// The playlist tree: the root folder holding one playlist, `name`, of every track, keyed by
/// its `TrackID`.
fn write_playlists<W: Write>(
    w: &mut Writer<W>,
    tracks: &[XmlTrack],
    name: &str,
) -> std::io::Result<()> {
    let entries = tracks.len().to_string();
    w.create_element("PLAYLISTS").write_inner_content(|w| {
        w.create_element("NODE")
            .with_attributes([("Type", "0"), ("Name", "ROOT"), ("Count", "1")])
            .write_inner_content(|w| {
                let playlist = w.create_element("NODE").with_attributes([
                    ("Name", name),
                    ("Type", "1"),
                    ("KeyType", "0"),
                    ("Entries", entries.as_str()),
                ]);
                if tracks.is_empty() {
                    playlist.write_empty()?;
                    return Ok(());
                }
                playlist.write_inner_content(|w| {
                    for id in 1..=tracks.len() {
                        let key = id.to_string();
                        w.create_element("TRACK")
                            .with_attribute(("Key", key.as_str()))
                            .write_empty()?;
                    }
                    Ok(())
                })?;
                Ok(())
            })?;
        Ok(())
    })?;
    Ok(())
}

/// The start of every XML SoundCheck writes, up to its product name: a file that starts
/// otherwise is not one of ours and is never replaced.
pub const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n  <PRODUCT Name=\"SoundCheck\" ";

/// Whether `head` (the first bytes of a file, at least [`XML_HEAD`]'s length when there are
/// that many) starts like an XML SoundCheck wrote.
#[must_use]
pub fn is_soundcheck_xml(head: &[u8]) -> bool {
    head.starts_with(XML_HEAD.as_bytes())
}

/// A batch's playlist name: [`PLAYLIST_PREFIX`] and the batch's local date and time, as its
/// folder is named (`SoundCheck 2026-10-11 14.05.33`).
#[must_use]
pub fn playlist_name(now: std::time::SystemTime) -> String {
    format!(
        "{PLAYLIST_PREFIX} {}",
        crate::artefacts::local_date_time(now)
    )
}

/// `TotalTime`: whole seconds, rounded down; 0 for a negative or undefined time.
fn whole_seconds(t: Seconds) -> u64 {
    // `as` saturates: NaN and negative values give 0.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let s = t.0.floor() as u64;
    s
}

/// `s` with every character XML 1.0 does not allow (C0 controls other than tab, line feed and
/// carriage return; U+FFFE, U+FFFF) replaced by U+FFFD. Tab, line feed and carriage return are
/// escaped in attributes by the writer.
fn text(s: &str) -> Cow<'_, str> {
    let allowed = |c: char| matches!(c, '\t' | '\n' | '\r' | ' '..='\u{FFFD}' | '\u{10000}'..);
    if s.chars().all(allowed) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(
            s.chars()
                .map(|c| if allowed(c) { c } else { '\u{FFFD}' })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests;
