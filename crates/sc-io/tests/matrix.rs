//! The file-layer fixture matrix: what "nothing lost" means, written down before the writers.
//!
//! Every fixture is a small synthetic file built in code from a fixed seed (no committed
//! binaries, no clock): WAV and RF64 per the Microsoft/IBM RIFF specification, WAVEFORMATEX(-
//! TENSIBLE), EBU Tech 3285 (`bext` v0/v1/v2) and EBU Tech 3306 (`ds64`); AIFF 1.3 and AIFF-C;
//! FLAC per RFC 9639 (a minimal writer with VERBATIM subframes, so no encoder is involved);
//! `ID3v2`.3 and `ID3v2`.4 tags per id3.org (extended headers, frame flags, tag-level
//! unsynchronisation), including GEOB objects shaped like Serato's (`Serato Markers2`,
//! `Serato BeatGrid`, `Serato Autotags`, after Holzhaus' published "serato-tags" layout) and the
//! matching `SERATO_*` Vorbis comments. Each fixture lists every block it contains (chunk, ID3
//! extended header and frame, FLAC metadata block, Vorbis field, leading and trailing tags) with
//! its SHA-256 and the fate a writer must give it: carried, patched, replaced or dropped (rules
//! in `matrix/cases.rs`, patch arithmetic in `matrix/expect.rs`).
//!
//! What runs now proves the matrix itself: symphonia decodes each fixture to the generated
//! samples (or refuses it for a documented reason), lofty and hound read what they support, an
//! independent reader (`matrix/parse.rs`, no `sc-io` code) lists exactly the expected blocks,
//! `sc_io::iff` lists the same chunks and reads the same samples (`matrix/walker.rs`, RF64
//! included), fixtures are small and deterministic, the golden manifest
//! `tests/fixtures/golden/file-layer-matrix.json` matches (exact; `UPDATE_GOLDEN=1` rewrites
//! it), and a reference writer passes every check while each injected mistake fails one
//! (`matrix/selftest.rs`). The writer tests (`matrix/writers.rs`) run the `sc-io` writers
//! (IFF render, ID3 edit) through the same runner and record each output's SHA-256 in the
//! manifest's `outputs` section, the FLAC render included; `flac -t` checks the FLAC outputs
//! with `SC_FLAC_TOOLS=1`. `matrix/real.rs` runs the same output checks on a folder of real
//! files through the write transaction when `SC_REAL_FIXTURES=<folder>` is set.

// A crate root resolves `mod x;` next to itself, so the modules are pointed at `matrix/`.
#[path = "matrix/aiff.rs"]
mod aiff;
#[path = "matrix/apply.rs"]
mod apply;
#[path = "matrix/cases.rs"]
mod cases;
#[path = "matrix/cases_flac.rs"]
mod cases_flac;
#[path = "matrix/cases_iff.rs"]
mod cases_iff;
#[path = "matrix/cases_real.rs"]
mod cases_real;
#[path = "matrix/check.rs"]
mod check;
#[path = "matrix/check_audio.rs"]
mod check_audio;
#[path = "matrix/decode.rs"]
mod decode;
#[path = "matrix/expect.rs"]
mod expect;
#[path = "matrix/flac.rs"]
mod flac;
#[path = "matrix/golden.rs"]
mod golden;
#[path = "matrix/id3.rs"]
mod id3;
#[path = "matrix/inspect.rs"]
mod inspect;
#[path = "matrix/literals.rs"]
mod literals;
#[path = "matrix/oracle.rs"]
mod oracle;
#[path = "matrix/parse.rs"]
mod parse;
#[path = "matrix/parse_id3.rs"]
mod parse_id3;
#[path = "matrix/pcm.rs"]
mod pcm;
#[path = "matrix/real.rs"]
mod real;
#[path = "matrix/riff.rs"]
mod riff;
#[path = "matrix/selftest.rs"]
mod selftest;
#[path = "matrix/walker.rs"]
mod walker;
#[path = "matrix/writers.rs"]
mod writers;

use std::io::Cursor;

use cases::{Symphonia, matrix};
use parse::{Container, Kind, parse};

/// Committed fixtures stay below 200 KB; the synthetic ones are held to the same budget.
const MAX_FIXTURE_BYTES: usize = 200 * 1024;

#[test]
fn every_fixture_decodes_with_symphonia_to_the_generated_samples() {
    for fx in matrix() {
        match (&fx.symphonia, decode::decode(&fx.bytes, fx.ext)) {
            (Symphonia::Decodes, Ok(d)) => {
                assert_eq!(
                    (d.sample_rate, d.channels, d.frames()),
                    (fx.sample_rate, fx.channels, fx.frames),
                    "{}",
                    fx.name
                );
                match &fx.source {
                    pcm::Samples::Int { bits, data } => {
                        assert!(d.as_bits(*bits) == *data, "{}: samples differ", fx.name);
                    }
                    pcm::Samples::Float(data) => {
                        let worst = d
                            .float
                            .iter()
                            .zip(data)
                            .map(|(a, b)| (a - b).abs())
                            .fold(0.0_f32, f32::max);
                        assert!(worst <= 1e-7, "{}: float error {worst}", fx.name);
                    }
                }
            }
            (Symphonia::Truncates { frames, .. }, Ok(d)) => {
                assert_eq!(
                    d.frames(),
                    *frames,
                    "{}: the documented truncation",
                    fx.name
                );
                let pcm::Samples::Int { bits, data } = &fx.source else {
                    panic!("{}: integer source expected", fx.name);
                };
                let channels = usize::from(fx.channels);
                assert!(d.as_bits(*bits) == data[..frames * channels], "{}", fx.name);
            }
            (Symphonia::Decodes | Symphonia::Truncates { .. }, Err(e)) => {
                panic!("{}: symphonia failed: {e}", fx.name)
            }
            (Symphonia::Refuses(_), Err(_)) => {}
            (Symphonia::Refuses(why), Ok(_)) => {
                panic!("{} now decodes; drop the documented quirk ({why})", fx.name)
            }
        }
    }
}

#[test]
fn iff_audio_chunks_hold_the_generated_samples() {
    for fx in matrix().iter().filter(|f| f.is_iff()) {
        let parsed = parse(&fx.bytes).unwrap_or_else(|e| panic!("{}: {e}", fx.name));
        let stored = inspect::iff_pcm(&parsed).unwrap_or_else(|e| panic!("{}: {e}", fx.name));
        assert_eq!(
            (stored.sample_rate, stored.channels, stored.frames),
            (fx.sample_rate, fx.channels, fx.frames),
            "{}",
            fx.name
        );
        assert!(stored.samples == fx.source, "{}: samples differ", fx.name);
    }
}

#[test]
fn independent_parser_lists_exactly_the_expected_blocks() {
    for fx in matrix() {
        let parsed = parse(&fx.bytes).unwrap_or_else(|e| panic!("{}: {e}", fx.name));
        assert_eq!(parsed.container, fx.container, "{}", fx.name);
        let want: Vec<_> = fx.expected.iter().map(|b| b.listed.clone()).collect();
        assert_eq!(parsed.listing(), want, "{}", fx.name);
    }
}

#[test]
fn fixtures_are_small_unique_and_deterministic() {
    let (a, b) = (matrix(), matrix());
    let mut names: Vec<_> = a.iter().map(|f| f.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), a.len(), "fixture names are unique");
    for (x, y) in a.iter().zip(&b) {
        assert!(
            x.bytes.len() < MAX_FIXTURE_BYTES,
            "{}: {} bytes",
            x.name,
            x.bytes.len()
        );
        assert!(x.bytes == y.bytes, "{} differs between two builds", x.name);
    }
}

#[test]
fn golden_manifest_matches_the_built_matrix() {
    golden::check_or_update(&matrix()).unwrap_or_else(|e| panic!("{e}"));
}

/// What lofty 0.25 reads of each fixture: the stream properties, a title from the tag the
/// fixture carries (`TIT2` or `TITLE`), and a picture where it holds `APIC`/`PICTURE`.
/// Documented quirks: lofty misdetects RF64 as MPEG and fails; with two ID3 chunks in one WAV
/// it reads the last one (so the picture check only applies to files with one tag).
#[test]
fn lofty_reads_the_tags_and_properties_it_supports() {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;
    for fx in matrix() {
        let probe = lofty::probe::Probe::new(Cursor::new(fx.bytes.clone()))
            .guess_file_type()
            .expect("probe");
        let read = probe.read();
        if fx.container == Container::Rf64 {
            assert!(read.is_err(), "{}: lofty now reads RF64", fx.name);
            continue;
        }
        let file = read.unwrap_or_else(|e| panic!("{}: lofty failed: {e}", fx.name));
        let props = file.properties();
        assert_eq!(props.sample_rate(), Some(fx.sample_rate), "{}", fx.name);
        let channels = props.channels().map(u16::from);
        assert_eq!(channels, Some(fx.channels), "{}", fx.name);
        let count = |kind: Kind, id: &str| {
            fx.expected
                .iter()
                .filter(|b| b.listed.kind == kind && b.listed.id == id)
                .count()
        };
        let titles: Vec<String> = file
            .tags()
            .iter()
            .filter_map(|t| t.title().map(|s| s.to_string()))
            .collect();
        if count(Kind::Id3Frame, "TIT2") + count(Kind::VorbisField, "TITLE") > 0 {
            let ok = titles
                .iter()
                .any(|t| t.starts_with("Matrix Tone") || t == "Mono");
            assert!(ok, "{}: titles {titles:?}", fx.name);
        }
        let id3_chunks = count(Kind::Chunk, "id3 ") + count(Kind::Chunk, "ID3 ");
        if id3_chunks <= 1 {
            // A leading ID3v2 tag in front of a FLAC stream is listed as one blob.
            let leading_apic = parse(&fx.bytes).is_ok_and(|p| {
                p.blocks
                    .iter()
                    .any(|b| b.kind == Kind::LeadingTag && b.bytes.windows(4).any(|w| w == b"APIC"))
            });
            let expect_picture = count(Kind::Id3Frame, "APIC") + count(Kind::FlacBlock, "PICTURE")
                > 0
                || leading_apic;
            let pictures: usize = file.tags().iter().map(|t| t.pictures().len()).sum();
            assert_eq!(
                pictures > 0,
                expect_picture,
                "{}: {pictures} pictures",
                fx.name
            );
        }
    }
}

/// Whether an odd-length chunk comes before `data`: hound 3.5 skips unknown chunks without
/// their RIFF pad byte and then fails ("Failed to read enough bytes"), a documented quirk.
fn odd_chunk_before_data(fx: &cases::Fixture) -> bool {
    fx.expected
        .iter()
        .filter(|b| b.listed.kind == Kind::Chunk)
        .take_while(|b| b.listed.id != "data")
        .any(|b| b.listed.pad.is_some())
}

/// hound 3.5 reads the `fmt ` of every RIFF/WAVE fixture it can: plain PCM, extensible and
/// float.
#[test]
fn hound_reads_the_fmt_of_every_riff_fixture() {
    for fx in matrix().iter().filter(|f| f.container == Container::Wave) {
        let read = hound::WavReader::new(Cursor::new(fx.bytes.clone()));
        if odd_chunk_before_data(fx) {
            assert!(read.is_err(), "{}: hound now honours pad bytes", fx.name);
            continue;
        }
        let reader = read.unwrap_or_else(|e| panic!("{}: hound failed: {e}", fx.name));
        let spec = reader.spec();
        let format = if fx.is_float() {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        };
        assert_eq!(
            (
                spec.channels,
                spec.sample_rate,
                spec.bits_per_sample,
                spec.sample_format
            ),
            (fx.channels, fx.sample_rate, u16::from(fx.bits), format),
            "{}",
            fx.name
        );
        assert_eq!(reader.duration() as usize, fx.frames, "{}", fx.name);
    }
}
