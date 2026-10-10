//! Unit tests of `crates/sc-io/src/rekordbox.rs` and `rekordbox/location.rs`. The expected XML
//! below is written by hand from the format document, not produced by the writer.
//! At 44.1 kHz, 128 BPM has a 20,671.875-sample beat; 120 BPM a 22,050-sample beat.
use super::*;
use sc_core::analysis::BeatUnit;
use sc_core::export::ExportedGrid;
use std::path::Path;

const SR: u32 = 44_100;

fn four_four() -> Meter {
    Meter::four_four()
}

fn tempo(bar1: u64, bpm: f64) -> Tempo {
    Tempo::of_bar1(SampleIndex(bar1), Bpm(bpm), &four_four(), SR).expect("4/4")
}

fn grid(bar1: u64, bpm: f64, meter: Meter) -> XmlGrid {
    XmlGrid::Grid {
        grid: ExportedGrid {
            bar1: SampleIndex(bar1),
            first_bar_line: SampleIndex(bar1),
            bpm: Bpm(bpm).written(),
            bpm_exact: Bpm(bpm),
            meter,
            edited: false,
            confirmed: false,
        },
        sample_rate: SR,
    }
}

/// A track without title and artist tags.
fn track(path: &str, seconds: f64, tempo: Tempo) -> XmlTrack {
    XmlTrack {
        path: PathBuf::from(path),
        title: None,
        artist: None,
        duration: Seconds(seconds),
        tempo,
    }
}

const PLAYLIST: &str = "SoundCheck 2026-10-11 14.05.33";

fn xml(tracks: &[XmlTrack]) -> String {
    String::from_utf8(xml_bytes(tracks, "9.9.9-test", PLAYLIST)).expect("UTF-8")
}

#[test]
fn inizio_is_lead_after_prepare() {
    // A Prepare cut leaves bar 1 between the lead (5 ms = 220.5 samples) and 1 ms plus a sample
    // after it: the first beat of the file is bar 1 itself, beat 1, at 5 or 6 ms.
    for (bar1, inizio) in [(221, "0.005"), (265, "0.006")] {
        let t = tempo(bar1, 120.0);
        assert_eq!(t.battito, 1);
        assert_eq!(format!("{:.3}", t.inizio.0), inizio);
        // Bar 1 shown further in (the cut was planned to the first bar line before it): the
        // grid is extrapolated back by whole bars to the same line.
        let later = tempo(bar1 + 8 * 88_200, 120.0);
        assert_eq!(later.battito, 1);
        assert!((later.inizio.0 - t.inizio.0).abs() < 1e-9, "{later:?}");
    }
    // At 48 kHz the lead is 240 samples.
    let t = Tempo::of_bar1(SampleIndex(240), Bpm(128.0), &four_four(), 48_000).expect("4/4");
    assert_eq!(
        (format!("{:.3}", t.inizio.0), t.battito),
        ("0.005".into(), 1)
    );
    // Through the exported grid as the sidecar records it.
    let from_grid = Tempo::of(&grid(221, 127.996, four_four())).expect("tempo");
    assert_eq!(format!("{:.2}", from_grid.bpm.0), "128.00");
    assert_eq!(from_grid.battito, 1);
    let text = xml(&[track("/m/a.wav", 300.0, from_grid)]);
    assert!(
        text.contains(r#"<TEMPO Inizio="0.005" Bpm="128.00" Metro="4/4" Battito="1"/>"#),
        "{text}"
    );
}

#[test]
fn battito_extrapolated() {
    // Bar 1 at 44,200 samples (1.0023 s) at 128 BPM: two beats fit before it, so the first beat
    // of the file is 2,856.25 samples in (0.065 s) and is beat 3 of the bar before bar 1.
    let t = tempo(44_200, 128.0);
    assert_eq!(
        (format!("{:.3}", t.inizio.0), t.battito),
        ("0.065".into(), 3)
    );
    // One, three and four beats back: beats 4, 2 and 1.
    for (beats_back, battito) in [(1_u32, 4_u8), (3, 2), (4, 1), (5, 4)] {
        let bar1 = 1_000 + u64::from(beats_back) * 22_050;
        let t = tempo(bar1, 120.0);
        assert_eq!(t.battito, battito, "{beats_back} beats back");
        assert!(
            (t.inizio.0 - 1_000.0 / f64::from(SR)).abs() < 1e-12,
            "{t:?}"
        );
    }
    // A bar line exactly at the start.
    let t = tempo(2 * 88_200, 120.0);
    assert_eq!((t.inizio.0, t.battito), (0.0, 1));
    // The written tempo, not the fitted one, lays the beats: 127.996 is written as 128.00.
    let fitted = Tempo::of_bar1(SampleIndex(44_200), Bpm(127.996), &four_four(), SR).expect("4/4");
    assert_eq!(fitted, tempo(44_200, 128.0));
}

#[test]
fn review_grid_has_no_tempo() {
    assert_eq!(
        Tempo::of(&XmlGrid::NeedsReview),
        Err(TempoWithheld::NeedsReview)
    );
    assert_eq!(Tempo::of(&XmlGrid::Absent), Err(TempoWithheld::Absent));
    // Odd meters are not written until rekordbox's reading of Metro is established.
    let seven = Meter::new(BeatUnit::Eighth, &[2, 2, 3]);
    assert_eq!(
        Tempo::of(&grid(221, 240.0, seven.clone())),
        Err(TempoWithheld::Meter(seven.to_string()))
    );
    assert!(matches!(
        Tempo::of(&grid(221, 120.0, Meter::three_four())),
        Err(TempoWithheld::Meter(_))
    ));
    assert_eq!(
        Tempo::of_bar1(SampleIndex(0), Bpm(0.001), &four_four(), SR),
        Err(TempoWithheld::InvalidTempo)
    );
    // Such a track cannot be listed at all: an `XmlTrack` always has a tempo.
}

/// A batch of two written by hand: a file tagged with a Turkish title and artist (`&` and `'`
/// in them, NFC) under a folder with `&`, and an untagged file named with `<`, `>` and `"`,
/// whose name falls back to the file name. Each `TRACK` has only `TrackID`, `Name`, `Artist`
/// (when tagged), `TotalTime`, `AverageBpm` and `Location`; the playlist is keyed by `TrackID`.
const EXPECTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="SoundCheck" Version="9.9.9-test" Company="SoundCheck"/>
  <COLLECTION Entries="2">
    <TRACK TrackID="1" Name="Deniz&apos;in Parçası" Artist="Ayşe &amp; İlhan" TotalTime="245" AverageBpm="128.00" Location="file://localhost/Music/%C5%9Eark%C4%B1%20%26%20Co/01.wav">
      <TEMPO Inizio="0.005" Bpm="128.00" Metro="4/4" Battito="1"/>
    </TRACK>
    <TRACK TrackID="2" Name="b &lt;live&gt; &quot;1&quot;" TotalTime="60" AverageBpm="120.00" Location="file://localhost/Music/b%20%3Clive%3E%20%221%22.wav">
      <TEMPO Inizio="0.065" Bpm="120.00" Metro="4/4" Battito="1"/>
    </TRACK>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Name="SoundCheck 2026-10-11 14.05.33" Type="1" KeyType="0" Entries="2">
        <TRACK Key="1"/>
        <TRACK Key="2"/>
      </NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
"#;

fn batch() -> Vec<XmlTrack> {
    vec![
        XmlTrack {
            title: Some("Deniz'in Parçası".into()),
            artist: Some("Ayşe & İlhan".into()),
            ..track("/Music/Şarkı & Co/01.wav", 245.97, tempo(221, 128.0))
        },
        // 2,866 samples = 0.065 s: bar 1 itself, at 120 BPM.
        track("/Music/b <live> \"1\".wav", 60.0, tempo(2_866, 120.0)),
    ]
}

#[test]
fn escapes() {
    assert_eq!(xml(&batch()), EXPECTED);
    // Characters XML 1.0 forbids never reach the file; the location keeps the bytes.
    let odd = XmlTrack {
        artist: Some("x\u{FFFF}".into()),
        ..track("/m/a\u{1}b.wav", 1.0, tempo(0, 120.0))
    };
    let text = xml(&[odd]);
    assert!(text.contains("Name=\"a\u{FFFD}b\""), "{text}");
    assert!(text.contains("Artist=\"x\u{FFFD}\""), "{text}");
    assert!(text.contains("/m/a%01b.wav"), "{text}");
    // Only the XML's key and the five attributes SoundCheck means to set.
    assert!(!EXPECTED.contains("Genre") && !EXPECTED.contains("Album"));
}

#[test]
fn deterministic_bytes() {
    let a = xml_bytes(&batch(), "9.9.9-test", PLAYLIST);
    let b = xml_bytes(&batch(), "9.9.9-test", PLAYLIST);
    assert_eq!(a, b);
    assert!(!a.contains(&b'\r'), "LF line ends only");
    assert!(a.ends_with(b"</DJ_PLAYLISTS>\n"));
    assert!(!a.starts_with(b"\xEF\xBB\xBF"), "no byte-order mark");
    // An empty batch is still a valid document with an empty playlist.
    let empty = String::from_utf8(xml_bytes(&[], "9.9.9-test", PLAYLIST)).expect("UTF-8");
    assert!(empty.contains(r#"<COLLECTION Entries="0"/>"#), "{empty}");
    assert!(
        empty.contains(
            r#"<NODE Name="SoundCheck 2026-10-11 14.05.33" Type="1" KeyType="0" Entries="0"/>"#
        ),
        "{empty}"
    );
}

#[test]
fn location_turkish_nfc() {
    // Precomposed (NFC) Turkish letters are encoded byte for byte: Ş C5 9E, ı C4 B1, ğ C4 9F,
    // ü C3 BC, ö C3 B6, ç C3 A7, İ C4 B0.
    let nfc = Path::new("/Müzik/Şarkı Ğüöçİ.wav");
    assert_eq!(
        location(nfc),
        "file://localhost/M%C3%BCzik/%C5%9Eark%C4%B1%20%C4%9E%C3%BC%C3%B6%C3%A7%C4%B0.wav"
    );
    // The decomposed spelling of the same name keeps its own bytes: S + U+0327 (CC A7),
    // u + U+0308 (CC 88). Nothing is normalised here; the caller passes the name as stored.
    let nfd = Path::new("/Mu\u{308}zik/S\u{327}ark\u{131}.wav");
    assert_eq!(
        location(nfd),
        "file://localhost/Mu%CC%88zik/S%CC%A7ark%C4%B1.wav"
    );
    for path in [nfc, nfd] {
        assert_eq!(decode_location(&location(path)).as_deref(), Some(path));
    }
    assert_eq!(decode_location("file:///a.wav"), None);
    assert_eq!(
        location(Path::new("rel/a.wav")),
        "file://localhost/rel/a.wav"
    );
}

/// The speller writes each name as its folder lists it. On macOS a file reached by either
/// Unicode spelling (or another case) gets the listed one: what the Finder, and rekordbox when it
/// added the file, see.
#[cfg(target_os = "macos")]
#[test]
fn location_uses_the_spelling_on_disk() {
    use unicode_normalization::UnicodeNormalization;
    let dir = tempfile::tempdir().expect("temp dir");
    let base = Speller::new().spell(dir.path());
    for stored in ["Şarkı İçin.wav", "Gu\u{308}l S\u{327}arkı.wav"] {
        std::fs::write(dir.path().join(stored), b"x").expect("write");
        // A speller reads each folder once, so a new one sees the file just written.
        let mut speller = Speller::new();
        let other: String = if stored.nfc().eq(stored.chars()) {
            stored.nfd().collect()
        } else {
            stored.nfc().collect()
        };
        assert_ne!(other, stored);
        for given in [other.clone(), other.to_ascii_uppercase(), stored.to_owned()] {
            assert_eq!(
                speller.spell(&dir.path().join(&given)),
                base.join(stored),
                "reached as {given:?}"
            );
        }
    }
}

#[test]
fn speller_drops_dots_and_keeps_what_it_cannot_list() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("folder");
    std::fs::write(dir.path().join("a.wav"), b"x").expect("write");
    let mut speller = Speller::new();
    let base = speller.spell(dir.path());
    assert_eq!(
        speller.spell(&dir.path().join("./sub/../a.wav")),
        base.join("a.wav")
    );
    // A name that is not there stays as given.
    assert_eq!(
        speller.spell(&dir.path().join("missing.wav")),
        base.join("missing.wav")
    );
    assert!(speller.spell(Path::new("rel.wav")).is_absolute());
}

/// One listing per folder per batch: 1,500 files in one folder are spelled with one read of it
/// and a map lookup each (about 0.1 s in a debug build, where folding the whole folder per file
/// took seconds).
#[test]
fn speller_reads_a_big_folder_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    let names: Vec<String> = (0..1_500).map(|i| format!("Şarkı {i:04}.wav")).collect();
    for n in &names {
        std::fs::write(dir.path().join(n), b"").expect("write");
    }
    let mut speller = Speller::new();
    let base = speller.spell(dir.path());
    for n in &names {
        assert_eq!(speller.spell(&dir.path().join(n)), base.join(n));
    }
    // The folder was listed once, however many of its files were spelled.
    assert_eq!(speller.listed_folders(), base.ancestors().count());
}

#[test]
fn parse_back() {
    use quick_xml::events::Event;
    let tracks = vec![
        track("/Music/Şarkı & Co/a.wav", 245.0, tempo(221, 128.0)),
        track(
            "/Music/S\u{327}ark\u{131}/b.aiff",
            61.9,
            tempo(44_200, 127.5),
        ),
        track("/Music/c%20d#e.flac", 10.0, tempo(5_000, 174.0)),
    ];
    let bytes = xml_bytes(&tracks, "9.9.9-test", "SoundCheck & <co>");
    let mut reader = quick_xml::Reader::from_reader(bytes.as_slice());
    let mut buf = Vec::new();
    let mut locations = Vec::new();
    let mut tempos: Vec<(usize, [String; 4])> = Vec::new();
    let mut keys = Vec::new();
    let mut in_playlists = false;
    loop {
        let event = reader.read_event_into(&mut buf).expect("well-formed XML");
        let e = match &event {
            Event::Start(e) | Event::Empty(e) => e.clone(),
            Event::Eof => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        let attr = |name: &str| {
            e.try_get_attribute(name).expect("attribute").map(|a| {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .expect("text")
                    .into_owned()
            })
        };
        match e.name().as_ref() {
            "PLAYLISTS" => in_playlists = true,
            "TRACK" if in_playlists => keys.push(attr("Key").expect("Key")),
            "TRACK" => locations.push(attr("Location").expect("Location")),
            "TEMPO" => tempos.push((
                locations.len(),
                ["Inizio", "Bpm", "Metro", "Battito"].map(|n| attr(n).expect("TEMPO attribute")),
            )),
            _ => {}
        }
        buf.clear();
    }
    // The playlist lists every track, keyed by its TrackID.
    assert_eq!(keys, ["1", "2", "3"]);
    let paths: Vec<PathBuf> = locations
        .iter()
        .map(|l| decode_location(l).expect("a file location"))
        .collect();
    let given: Vec<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
    assert_eq!(paths, given);
    assert_eq!(tempos.len(), 3);
    for (index, [inizio, bpm, metro, battito]) in &tempos {
        let t = tracks[index - 1].tempo;
        let inizio: f64 = inizio.parse().expect("number");
        assert!((inizio - t.inizio.0).abs() <= 0.0005, "{inizio} vs {t:?}");
        assert_eq!(bpm.parse::<f64>().expect("number"), t.bpm.0);
        assert_eq!(metro, "4/4");
        assert_eq!(battito.parse::<u8>().expect("number"), t.battito);
    }
}
