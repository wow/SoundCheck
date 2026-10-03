//! `sc_io::iff` against the matrix: on every WAV/RF64/AIFF/AIFF-C fixture the walker lists the
//! same chunks as the independent reader in `parse.rs` (id, header offset, payload bytes, pad
//! byte value, trailing bytes), and the PCM reader returns the samples the fixture was built
//! from (exact for integers, bit-exact for floats), including RF64 (which symphonia refuses),
//! the odd `data` chunk without a pad byte followed by an `ID3v1` tag, and an `SSND` offset.

use std::io::Cursor;
use std::path::Path;

use sc_io::iff::{self, ChunkTable, Container as IffContainer, SampleEncoding};

use super::cases::{Fixture, matrix};
use super::parse::{Container, Kind, parse};
use super::pcm::Samples;

fn bytes_of(fx: &Fixture, range: &std::ops::Range<u64>) -> Vec<u8> {
    let start = usize::try_from(range.start).expect("small fixture");
    let end = usize::try_from(range.end).expect("small fixture");
    fx.bytes[start..end].to_vec()
}

fn header(fx: &Fixture) -> iff::IffHeader {
    iff::read_header(&mut Cursor::new(&fx.bytes), Path::new(fx.name))
        .unwrap_or_else(|e| panic!("{}: {e}", fx.name))
}

/// (id, header offset, payload bytes, pad) per chunk, and the trailing bytes.
type Listing = (Vec<(String, u64, Vec<u8>, Option<u8>)>, Option<Vec<u8>>);

fn walker_listing(fx: &Fixture, t: &ChunkTable) -> Listing {
    let chunks = t
        .chunks
        .iter()
        .map(|c| {
            (
                c.id_text(),
                c.header_offset,
                bytes_of(fx, &c.payload),
                c.pad,
            )
        })
        .collect();
    (chunks, t.trailing.as_ref().map(|r| bytes_of(fx, r)))
}

fn parser_listing(fx: &Fixture) -> Listing {
    let parsed = parse(&fx.bytes).unwrap_or_else(|e| panic!("{}: {e}", fx.name));
    let chunks = parsed
        .blocks
        .iter()
        .filter(|b| b.kind == Kind::Chunk)
        .map(|b| (b.id.clone(), b.offset as u64, b.bytes.clone(), b.pad))
        .collect();
    let trailing = parsed
        .blocks
        .iter()
        .find(|b| b.kind == Kind::Trailing)
        .map(|b| b.bytes.clone());
    (chunks, trailing)
}

fn iff_fixtures() -> Vec<Fixture> {
    matrix().into_iter().filter(Fixture::is_iff).collect()
}

#[test]
fn walker_lists_the_same_chunks_as_the_independent_parser() {
    let fixtures = iff_fixtures();
    assert_eq!(fixtures.len(), 20, "WAV/RF64/AIFF/AIFF-C fixtures");
    for fx in &fixtures {
        let t = header(fx).table;
        let want = match fx.container {
            Container::Wave => IffContainer::Riff,
            Container::Rf64 => IffContainer::Rf64,
            Container::Aiff => IffContainer::Aiff,
            Container::Aifc => IffContainer::Aifc,
            Container::Flac => unreachable!("filtered"),
        };
        assert_eq!(t.container, want, "{}", fx.name);
        assert_eq!(walker_listing(fx, &t), parser_listing(fx), "{}", fx.name);
        assert!(!t.truncated, "{}", fx.name);
        assert_eq!(t.file_len, fx.bytes.len() as u64, "{}", fx.name);
        for c in &t.chunks {
            let odd = c.size_declared % 2 == 1;
            assert_eq!(c.pad_missing, odd && c.pad.is_none(), "{}", fx.name);
        }
    }
}

#[test]
fn walker_handles_the_documented_quirks() {
    let fixtures = iff_fixtures();
    let get = |name: &str| fixtures.iter().find(|f| f.name == name).expect(name);

    let odd = get("wav24-mono-odd-data-no-pad-id3v1");
    let t = header(odd).table;
    let data = t.find(b"data").expect("data");
    assert!(data.pad_missing && data.pad.is_none());
    assert_eq!(t.container_end(), data.payload.end);
    let trailing = t.trailing.clone().expect("ID3v1 after the RIFF end");
    assert_eq!(trailing.end - trailing.start, 128);
    assert!(bytes_of(odd, &trailing).starts_with(b"TAG"));

    let rf64 = header(get("rf64-24-48k"));
    let ds64 = rf64.table.ds64.as_ref().expect("ds64");
    assert_eq!(ds64.sample_count, rf64.format.frames);
    assert_eq!(
        rf64.table.find(b"data").expect("data").size_declared,
        ds64.data_size
    );

    let offset = header(get("aiff16-mono-ssnd-offset"));
    let ssnd = offset.table.find(b"SSND").expect("SSND");
    assert_eq!(offset.format.data.start, ssnd.payload.start + 8 + 12);
    assert!(!offset.format.frames_mismatch);

    let pad = header(get("wav24-bwf-ixml-cue-smpl-id3v24")).table;
    assert_eq!(pad.find(b"xodd").expect("xodd").pad, Some(0x20));
}

#[test]
fn pcm_reader_returns_the_generated_samples() {
    for fx in iff_fixtures() {
        let h = header(&fx);
        let f = &h.format;
        assert_eq!(
            (f.sample_rate, f.channels, f.frames),
            (fx.sample_rate, fx.channels, fx.frames as u64),
            "{}",
            fx.name
        );
        assert_eq!(
            f.frames_declared.is_some(),
            matches!(fx.container, Container::Aiff | Container::Aifc),
            "{}",
            fx.name
        );
        let path = Path::new(fx.name);
        match &fx.source {
            Samples::Int { bits, data } => {
                assert_eq!(f.valid_bits, u16::from(*bits), "{}", fx.name);
                let got = iff::read_all_int(Cursor::new(&fx.bytes), f, path)
                    .unwrap_or_else(|e| panic!("{}: {e}", fx.name));
                assert!(got == *data, "{}: integer samples differ", fx.name);
            }
            Samples::Float(data) => {
                assert_eq!(f.encoding, SampleEncoding::FloatLe32, "{}", fx.name);
                let got = iff::read_all_float(Cursor::new(&fx.bytes), f, path)
                    .unwrap_or_else(|e| panic!("{}: {e}", fx.name));
                assert_eq!(got.len(), data.len(), "{}", fx.name);
                for (g, w) in got.iter().zip(data) {
                    // Every stored f32 widens to f64 exactly, so narrowing back is lossless.
                    #[allow(clippy::cast_possible_truncation)]
                    let narrowed = *g as f32;
                    assert_eq!(narrowed.to_bits(), w.to_bits(), "{}", fx.name);
                }
            }
        }
    }
}
