//! Reading back an XML SoundCheck wrote, so a later batch into the same folder can keep what it
//! lists: every field of a track comes back as [`super::write_xml`] writes it, so an entry that is
//! kept comes out with the same attributes.

use quick_xml::events::{BytesStart, Event};

use sc_core::{Bpm, Seconds};

use super::{Tempo, XmlTrack, decode_location};

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    let found = e.try_get_attribute(name).ok()??;
    let value = found
        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
        .ok()?;
    Some(value.into_owned())
}

/// The track a collection `TRACK` element starts, without its tempo yet.
fn track_of(e: &BytesStart<'_>) -> Option<XmlTrack> {
    Some(XmlTrack {
        path: decode_location(&attr(e, "Location")?)?,
        title: attr(e, "Name"),
        artist: attr(e, "Artist"),
        duration: Seconds(attr(e, "TotalTime")?.parse().ok()?),
        tempo: Tempo {
            inizio: Seconds(0.0),
            bpm: Bpm(0.0),
            battito: 1,
        },
    })
}

fn tempo_of(e: &BytesStart<'_>) -> Option<Tempo> {
    Some(Tempo {
        inizio: Seconds(attr(e, "Inizio")?.parse().ok()?),
        bpm: Bpm(attr(e, "Bpm")?.parse().ok()?),
        battito: attr(e, "Battito")?.parse().ok()?,
    })
}

/// The tracks of the XML `bytes`, in order; `None` when it is not readable whole as one SoundCheck
/// wrote: cut short, or a track without the `TEMPO` every listed track has. A caller must not take
/// that for an empty list: writing over it would lose what it lists.
#[must_use]
pub fn read_tracks(bytes: &[u8]) -> Option<Vec<XmlTrack>> {
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buf = Vec::new();
    let mut tracks = Vec::new();
    let mut in_collection = false;
    let mut current: Option<XmlTrack> = None;
    let mut has_tempo = false;
    let mut ended = false;
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(e) if e.name().as_ref() == "COLLECTION" => in_collection = true,
            Event::End(e) if e.name().as_ref() == "COLLECTION" => in_collection = false,
            Event::Start(e) if in_collection && e.name().as_ref() == "TRACK" => {
                current = Some(track_of(&e)?);
                has_tempo = false;
            }
            Event::Empty(e) if current.is_some() && e.name().as_ref() == "TEMPO" => {
                if let Some(track) = &mut current {
                    track.tempo = tempo_of(&e)?;
                    has_tempo = true;
                }
            }
            Event::End(e) if in_collection && e.name().as_ref() == "TRACK" => {
                if !has_tempo {
                    return None;
                }
                tracks.push(current.take()?);
            }
            Event::Empty(e) if in_collection && e.name().as_ref() == "TRACK" => return None,
            Event::End(e) if e.name().as_ref() == "DJ_PLAYLISTS" => ended = true,
            Event::Eof => return ended.then_some(tracks),
            _ => {}
        }
        buf.clear();
    }
}
