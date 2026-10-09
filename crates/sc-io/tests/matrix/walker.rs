//! `sc_io::iff` against the matrix: on every WAV/RF64/AIFF/AIFF-C fixture the walker lists the
//! same chunks as the independent reader in `parse.rs` (id, header offset, payload bytes, pad
//! byte value, trailing bytes), the decoded format agrees field by field with the independent
//! header inspection in `inspect.rs`, and the PCM reader returns the samples the fixture was
//! built from (exact for integers, bit-exact for floats, no padding bits set), including RF64
//! (which symphonia refuses), the odd `data` chunk without a pad byte followed by an `ID3v1`
//! tag, and an `SSND` offset.

use std::io::Cursor;
use std::ops::Range;
use std::path::Path;

use sc_io::iff::SampleEncoding;
use sc_io::iff::{self, AudioFormat, ChunkTable, Container as IffContainer, PcmReader};

use super::cases::{Fixture, matrix};
use super::inspect::{aiff_comm, wav_fmt};
use super::parse::{Block, Container, Kind, Parsed, parse};
use super::pcm::Samples;

fn bytes_of(fx: &Fixture, range: &Range<u64>) -> Vec<u8> {
    let start = usize::try_from(range.start).expect("small fixture");
    let end = usize::try_from(range.end).expect("small fixture");
    fx.bytes[start..end].to_vec()
}

fn header(fx: &Fixture) -> iff::IffHeader {
    iff::read_header(&mut Cursor::new(&fx.bytes), Path::new(fx.name))
        .unwrap_or_else(|e| panic!("{}: {e}", fx.name))
}

fn parsed(fx: &Fixture) -> Parsed {
    parse(&fx.bytes).unwrap_or_else(|e| panic!("{}: {e}", fx.name))
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

fn parser_listing(p: &Parsed) -> Listing {
    let chunks = p
        .blocks
        .iter()
        .filter(|b| b.kind == Kind::Chunk)
        .map(|b| (b.id.clone(), b.offset as u64, b.bytes.clone(), b.pad))
        .collect();
    let trailing = p
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
    assert_eq!(fixtures.len(), 30, "WAV/RF64/AIFF/AIFF-C fixtures");
    let mut pads_missing = Vec::new();
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
        let p = parsed(fx);
        assert_eq!(walker_listing(fx, &t), parser_listing(&p), "{}", fx.name);
        assert!(!t.truncated, "{}", fx.name);
        assert_eq!(t.file_len, fx.bytes.len() as u64, "{}", fx.name);
        // Stray bytes the container size counts lie between the last chunk and its end.
        assert_eq!(
            t.container_end(),
            t.chunks.last().map_or(12, sc_io::iff::Chunk::end) + p.stray_in_container as u64,
            "{}",
            fx.name
        );
        for c in &t.chunks {
            assert!(!c.beyond_container, "{}: {}", fx.name, c.id_text());
            let rf64_size = t.container == IffContainer::Rf64 && &c.id == b"data";
            let field = if rf64_size {
                u32::MAX
            } else {
                u32::try_from(c.size_declared).expect("small")
            };
            assert_eq!(c.size_field, field, "{}: {}", fx.name, c.id_text());
            if c.pad_missing {
                pads_missing.push((fx.name, c.id_text()));
            }
        }
    }
    // The odd chunks written without their pad byte.
    assert_eq!(
        pads_missing,
        [
            ("wav24-mono-odd-data-no-pad-id3v1", "data".to_string()),
            ("aiff16-name-anno-id3v24-odd-unpadded", "ID3 ".to_string()),
        ]
    );
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

    // Bytes after the last chunk, inside or after the FORM, are trailing: (stray bytes, bytes
    // the FORM size counts) per fixture.
    for (name, stray, inside) in [
        ("aiff16-id3v24-2-stray-in-form", 2, 2),
        ("aiff16-comt-comm-id3v23-2-after-form", 2, 0),
        ("aiff16-name-copyright-id3v23-1-stray-in-form", 1, 1),
    ] {
        let fx = get(name);
        let t = header(fx).table;
        let len = fx.bytes.len() as u64;
        assert_eq!(t.trailing, Some(len - stray..len), "{name}");
        assert!(
            bytes_of(fx, &(len - stray..len)).iter().all(|b| *b == 0),
            "{name}"
        );
        assert_eq!(t.container_end(), len - stray + inside, "{name}");
        let id3 = t.chunks.last().expect("chunks");
        assert_eq!(
            (&id3.id, id3.pad, id3.end()),
            (b"ID3 ", None, len - stray),
            "{name}"
        );
    }
    let unpadded = get("aiff16-name-anno-id3v24-odd-unpadded");
    let t = header(unpadded).table;
    let id3 = t.chunks.last().expect("chunks");
    assert!(
        id3.payload_len() % 2 == 1 && id3.pad_missing,
        "odd ID3 chunk without its pad"
    );
    assert_eq!(t.container_end(), unpadded.bytes.len() as u64);
    assert_eq!(t.trailing, None);
}

fn chunk<'p>(p: &'p Parsed, id: &str, fx: &Fixture) -> &'p Block {
    p.find(Kind::Chunk, id)
        .unwrap_or_else(|| panic!("{}: no {id}", fx.name))
}

/// The format the independent inspection (`inspect.rs` on `parse.rs` blocks) gives.
fn expected_format(fx: &Fixture, t: &ChunkTable) -> AudioFormat {
    let p = parsed(fx);
    let index = |id: &[u8; 4]| t.position(id).expect("chunk present");
    if matches!(fx.container, Container::Wave | Container::Rf64) {
        let raw = &chunk(&p, "fmt ", fx).bytes;
        let fmt = wav_fmt(raw).unwrap_or_else(|e| panic!("{}: {e}", fx.name));
        let float = fmt.format_tag == 3 || fmt.sub_format == Some(3);
        let extensible = fmt.format_tag == 0xFFFE;
        let le16 = |at: usize| u16::from_le_bytes([raw[at], raw[at + 1]]);
        let data = chunk(&p, "data", fx);
        let start = data.offset as u64 + 8;
        AudioFormat {
            sample_rate: fmt.sample_rate,
            channels: fmt.channels,
            bits_per_sample: fmt.bits,
            valid_bits: if extensible { le16(18) } else { fmt.bits },
            encoding: match (float, fmt.bits) {
                (true, _) => SampleEncoding::FloatLe32,
                (false, 8) => SampleEncoding::UnsignedInt8,
                (false, _) => SampleEncoding::IntLe,
            },
            block_align: fmt.block_align,
            frames: fx.frames as u64,
            frames_declared: None,
            frames_mismatch: false,
            data: start..start + data.bytes.len() as u64,
            channel_mask: extensible
                .then(|| u32::from_le_bytes([raw[20], raw[21], raw[22], raw[23]])),
            format_tag: Some(fmt.format_tag),
            aifc_compression: None,
            format_chunk: index(b"fmt "),
            audio_chunk: Some(index(b"data")),
        }
    } else {
        let aifc = fx.container == Container::Aifc;
        let comm = aiff_comm(&chunk(&p, "COMM", fx).bytes, aifc)
            .unwrap_or_else(|e| panic!("{}: {e}", fx.name));
        let ssnd = chunk(&p, "SSND", fx);
        let offset =
            u32::from_be_bytes([ssnd.bytes[0], ssnd.bytes[1], ssnd.bytes[2], ssnd.bytes[3]]);
        let start = ssnd.offset as u64 + 8 + 8 + u64::from(offset);
        let container = comm.bits.div_ceil(8) * 8;
        let block_align = comm.channels * container / 8;
        // Rates in the matrix are whole numbers, which f64 holds exactly.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rate = comm.sample_rate as u32;
        AudioFormat {
            sample_rate: rate,
            channels: comm.channels,
            bits_per_sample: container,
            valid_bits: comm.bits,
            encoding: if comm.compression == Some(*b"sowt") {
                SampleEncoding::IntLe
            } else {
                SampleEncoding::IntBe
            },
            block_align,
            frames: u64::from(comm.frames),
            frames_declared: Some(u64::from(comm.frames)),
            frames_mismatch: false,
            data: start..start + u64::from(comm.frames) * u64::from(block_align),
            channel_mask: None,
            format_tag: None,
            aifc_compression: comm.compression,
            format_chunk: index(b"COMM"),
            audio_chunk: Some(index(b"SSND")),
        }
    }
}

#[test]
fn format_agrees_with_the_independent_header_inspection() {
    for fx in iff_fixtures() {
        let h = header(&fx);
        assert_eq!(h.format, expected_format(&fx, &h.table), "{}", fx.name);
        let audio = &h.table.chunks[h.format.audio_chunk.expect("audio chunk")];
        assert!(audio.payload.start <= h.format.data.start, "{}", fx.name);
        assert!(h.format.data.end <= audio.payload.end, "{}", fx.name);
    }
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
        let mut pcm = PcmReader::new(Cursor::new(&fx.bytes), f, Path::new(fx.name))
            .unwrap_or_else(|e| panic!("{}: {e}", fx.name));
        match &fx.source {
            Samples::Int { bits, data } => {
                assert_eq!(f.valid_bits, u16::from(*bits), "{}", fx.name);
                let (mut got, mut block) = (Vec::new(), Vec::new());
                while pcm.next_block_int(&mut block).expect("read") > 0 {
                    got.extend_from_slice(&block);
                }
                assert!(got == *data, "{}: integer samples differ", fx.name);
                assert_eq!(pcm.padding_bits_nonzero(), 0, "{}", fx.name);
            }
            Samples::Float(data) => {
                let (mut got, mut block) = (Vec::new(), Vec::new());
                while pcm.next_block_float(&mut block).expect("read") > 0 {
                    got.extend_from_slice(&block);
                }
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
