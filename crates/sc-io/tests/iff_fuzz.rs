//! Property tests for `sc_io::iff` (256 cases each by default; `PROPTEST_CASES` raises it).
//!
//! 1. Random valid chunk soups (RIFF, RF64 with `ds64`, FORM AIFF; random printable ids,
//!    payloads of 0..300 bytes including odd ones with random pad values, `fmt `/`data` or
//!    `COMM`/`SSND` at random places, an optional missing pad after an odd last chunk, optional
//!    stray bytes after the container): the walk returns exactly the generator's record and the
//!    PCM reader returns the generated samples.
//! 2. Arbitrary bytes (raw or behind a valid container header), and random truncations and
//!    single-byte mutations of the matrix fixtures: walk, format and PCM never panic, never read
//!    or seek past the end of the input (the reader panics if they do), and every range and
//!    buffer they produce lies within the input length.

#[allow(dead_code)]
#[path = "matrix/aiff.rs"]
mod aiff;
#[allow(dead_code)]
#[path = "matrix/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "matrix/cases_flac.rs"]
mod cases_flac;
#[allow(dead_code)]
#[path = "matrix/cases_iff.rs"]
mod cases_iff;
#[allow(dead_code)]
#[path = "matrix/flac.rs"]
mod flac;
#[allow(dead_code)]
#[path = "matrix/id3.rs"]
mod id3;
#[allow(dead_code, unused_imports)]
#[path = "matrix/parse.rs"]
mod parse;
#[allow(dead_code)]
#[path = "matrix/parse_id3.rs"]
mod parse_id3;
#[allow(dead_code)]
#[path = "matrix/pcm.rs"]
mod pcm;
#[allow(dead_code)]
#[path = "matrix/riff.rs"]
mod riff;

use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

use proptest::prelude::*;
use proptest::sample::Index;
use sc_io::iff::{self, ChunkTable, Container, PcmReader};

/// A reader over a byte slice that panics on any read or seek past its end.
struct Strict<'a> {
    bytes: &'a [u8],
    pos: u64,
}

impl<'a> Strict<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl Read for Strict<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.bytes.len() as u64;
        let end = self.pos + buf.len() as u64;
        assert!(end <= len, "read {}..{end} past the end ({len})", self.pos);
        let start = usize::try_from(self.pos).expect("small input");
        buf.copy_from_slice(&self.bytes[start..start + buf.len()]);
        self.pos = end;
        Ok(buf.len())
    }
}

impl Seek for Strict<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let len = self.bytes.len() as u64;
        let target = match from {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let target = target.expect("seek before the start");
        assert!(target <= len, "seek to {target} past the end ({len})");
        self.pos = target;
        Ok(target)
    }
}

const PATH: &str = "fuzz";

/// Ids the soups never use for their random chunks (the format and audio chunks are placed by
/// the generator).
const RESERVED: [&[u8; 4]; 5] = [b"fmt ", b"data", b"COMM", b"SSND", b"ds64"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoupKind {
    Riff,
    Rf64,
    Aiff,
}

#[derive(Debug, Clone)]
struct SoupChunk {
    id: [u8; 4],
    payload: Vec<u8>,
    pad: u8,
}

#[derive(Debug, Clone)]
struct Soup {
    kind: SoupKind,
    chunks: Vec<SoupChunk>,
    fmt_at: Index,
    data_at: Index,
    samples: Vec<i16>,
    ssnd_offset: u8,
    omit_last_pad: bool,
    trailing: Vec<u8>,
}

/// One chunk as written: id, header offset, payload range, pad byte.
type Written = ([u8; 4], u64, Range<u64>, Option<u8>);

/// What the generator wrote: every chunk and the trailing bytes.
#[derive(Debug, PartialEq, Eq)]
struct Record {
    chunks: Vec<Written>,
    trailing: Option<Range<u64>>,
}

fn chunk_id() -> impl Strategy<Value = [u8; 4]> {
    (b'!'..=b'~', b' '..=b'~', b' '..=b'~', b' '..=b'~').prop_map(|(a, b, c, d)| {
        let id = [a, b, c, d];
        if RESERVED.contains(&&id) {
            [b'x', b, c, d]
        } else {
            id
        }
    })
}

fn soup() -> impl Strategy<Value = Soup> {
    let chunk = (
        chunk_id(),
        prop::collection::vec(any::<u8>(), 0..300),
        any::<u8>(),
    )
        .prop_map(|(id, payload, pad)| SoupChunk { id, payload, pad });
    (
        prop_oneof![
            Just(SoupKind::Riff),
            Just(SoupKind::Rf64),
            Just(SoupKind::Aiff)
        ],
        prop::collection::vec(chunk, 0..10),
        any::<Index>(),
        any::<Index>(),
        prop::collection::vec(any::<i16>(), 0..120),
        0_u8..16,
        any::<bool>(),
        prop::collection::vec(any::<u8>(), 0..40),
    )
        .prop_map(
            |(kind, chunks, fmt_at, data_at, mut samples, ssnd_offset, omit_last_pad, trailing)| {
                samples.truncate(samples.len() / 2 * 2);
                Soup {
                    kind,
                    chunks,
                    fmt_at,
                    data_at,
                    samples,
                    ssnd_offset,
                    omit_last_pad,
                    trailing,
                }
            },
        )
}

fn put_size(out: &mut Vec<u8>, size: u32, big: bool) {
    out.extend_from_slice(&if big {
        size.to_be_bytes()
    } else {
        size.to_le_bytes()
    });
}

fn small(n: usize) -> u32 {
    u32::try_from(n).expect("soups are small")
}

/// The ordered chunk list of a soup: random chunks with the format and audio chunks inserted.
fn soup_chunks(s: &Soup) -> Vec<SoupChunk> {
    let big = s.kind == SoupKind::Aiff;
    let frames = s.samples.len() / 2;
    let (format, audio) = if big {
        let mut comm = 2_u16.to_be_bytes().to_vec();
        comm.extend_from_slice(&small(frames).to_be_bytes());
        comm.extend_from_slice(&16_u16.to_be_bytes());
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        let mut ssnd = u32::from(s.ssnd_offset).to_be_bytes().to_vec();
        ssnd.extend_from_slice(&[0; 4]);
        ssnd.extend(std::iter::repeat_n(0xA5, usize::from(s.ssnd_offset)));
        ssnd.extend(s.samples.iter().flat_map(|v| v.to_be_bytes()));
        ((*b"COMM", comm), (*b"SSND", ssnd))
    } else {
        let mut fmt = 1_u16.to_le_bytes().to_vec();
        fmt.extend_from_slice(&2_u16.to_le_bytes());
        fmt.extend_from_slice(&44_100_u32.to_le_bytes());
        fmt.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
        fmt.extend_from_slice(&4_u16.to_le_bytes());
        fmt.extend_from_slice(&16_u16.to_le_bytes());
        let data = s.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
        ((*b"fmt ", fmt), (*b"data", data))
    };
    let mut chunks = s.chunks.clone();
    let at = s.fmt_at.index(chunks.len() + 1);
    chunks.insert(
        at,
        SoupChunk {
            id: format.0,
            payload: format.1,
            pad: 0,
        },
    );
    let at = s.data_at.index(chunks.len() + 1);
    chunks.insert(
        at,
        SoupChunk {
            id: audio.0,
            payload: audio.1,
            pad: 0,
        },
    );
    chunks
}

/// Writes a soup; returns the file and the record of what was written.
fn build(s: &Soup) -> (Vec<u8>, Record) {
    let big = s.kind == SoupKind::Aiff;
    let rf64 = s.kind == SoupKind::Rf64;
    let chunks = soup_chunks(s);
    let mut body = Vec::new();
    let mut record = Vec::new();
    let base: u64 = if rf64 { 12 + 8 + 28 } else { 12 };
    let mut data_len = 0_u64;
    for (i, c) in chunks.iter().enumerate() {
        let header = base + body.len() as u64;
        let len = c.payload.len();
        body.extend_from_slice(&c.id);
        let is_data = &c.id == b"data";
        put_size(
            &mut body,
            if rf64 && is_data {
                u32::MAX
            } else {
                small(len)
            },
            big,
        );
        body.extend_from_slice(&c.payload);
        let last = i + 1 == chunks.len();
        let pad = (len % 2 == 1 && !(last && s.omit_last_pad)).then_some(c.pad);
        if let Some(p) = pad {
            body.push(p);
        }
        if is_data {
            data_len = len as u64;
        }
        record.push((c.id, header, header + 8..header + 8 + len as u64, pad));
    }
    let mut out = Vec::new();
    match s.kind {
        SoupKind::Riff => {
            out.extend_from_slice(b"RIFF");
            put_size(&mut out, small(4 + body.len()), false);
            out.extend_from_slice(b"WAVE");
        }
        SoupKind::Aiff => {
            out.extend_from_slice(b"FORM");
            put_size(&mut out, small(4 + body.len()), true);
            out.extend_from_slice(b"AIFF");
        }
        SoupKind::Rf64 => {
            out.extend_from_slice(b"RF64");
            put_size(&mut out, u32::MAX, false);
            out.extend_from_slice(b"WAVE");
            out.extend_from_slice(b"ds64");
            put_size(&mut out, 28, false);
            out.extend_from_slice(&((4 + 8 + 28 + body.len()) as u64).to_le_bytes());
            out.extend_from_slice(&data_len.to_le_bytes());
            out.extend_from_slice(&(data_len / 4).to_le_bytes());
            out.extend_from_slice(&0_u32.to_le_bytes());
            record.insert(0, (*b"ds64", 12, 20..48, None));
        }
    }
    out.extend_from_slice(&body);
    let end = out.len() as u64;
    out.extend_from_slice(&s.trailing);
    let trailing = (!s.trailing.is_empty()).then_some(end..out.len() as u64);
    (
        out,
        Record {
            chunks: record,
            trailing,
        },
    )
}

fn record_of(t: &ChunkTable) -> Record {
    Record {
        chunks: t
            .chunks
            .iter()
            .map(|c| (c.id, c.header_offset, c.payload.clone(), c.pad))
            .collect(),
        trailing: t.trailing.clone(),
    }
}

/// Walk, format and PCM on arbitrary input: checks every bound it can.
fn exercise(bytes: &[u8]) {
    let len = bytes.len() as u64;
    let path = Path::new(PATH);
    let Ok(table) = iff::walk(&mut Strict::new(bytes), path) else {
        return;
    };
    assert_eq!(table.file_len, len);
    assert!(
        table.chunks.len() as u64 * 8 <= len,
        "chunk count bounded by the input"
    );
    if let Some(d) = &table.ds64 {
        assert!(
            d.table.len() as u64 * 12 <= len,
            "ds64 table bounded by the input"
        );
    }
    let mut prev_end = 12;
    for c in &table.chunks {
        assert!(
            c.header_offset >= prev_end,
            "chunks in order without overlap"
        );
        assert_eq!(c.payload.start, c.header_offset + 8);
        assert!(c.payload.start <= c.payload.end && c.end() <= len);
        assert!(c.payload_len() <= c.size_declared);
        assert_eq!(c.pad_missing, c.size_declared % 2 == 1 && c.pad.is_none());
        prev_end = c.end();
    }
    if let Some(t) = &table.trailing {
        assert!(t.start >= prev_end && t.start < t.end && t.end == len);
    }
    let Ok(format) = iff::read_format(&mut Strict::new(bytes), &table, path) else {
        return;
    };
    assert!(format.data.start <= format.data.end && format.data.end <= len);
    assert_eq!(
        format.data.end - format.data.start,
        format.frames * u64::from(format.block_align)
    );
    let mut pcm =
        PcmReader::new(Strict::new(bytes), &format, path).expect("a read format is consistent");
    assert!(
        pcm.buffer_len_bytes() as u64 <= len,
        "buffer bounded by the input"
    );
    let mut frames = 0_u64;
    if pcm.is_float() {
        let mut out = Vec::new();
        while let Ok(n @ 1..) = pcm.next_block_float(&mut out) {
            assert!(out.len() as u64 <= len);
            frames += n as u64;
        }
    } else {
        let mut out = Vec::new();
        while let Ok(n @ 1..) = pcm.next_block_int(&mut out) {
            assert!(out.len() as u64 <= len);
            frames += n as u64;
        }
    }
    assert_eq!(frames, format.frames);
}

static FIXTURES: LazyLock<Vec<Vec<u8>>> = LazyLock::new(|| {
    cases::matrix()
        .into_iter()
        .filter(cases::Fixture::is_iff)
        .map(|f| f.bytes)
        .collect()
});

fn container_header() -> impl Strategy<Value = Vec<u8>> {
    (
        prop_oneof![Just(&b"RIFF"[..]), Just(&b"RF64"[..]), Just(&b"FORM"[..]),],
        prop_oneof![Just(&b"WAVE"[..]), Just(&b"AIFF"[..]), Just(&b"AIFC"[..])],
        any::<u32>(),
        prop::collection::vec(any::<u8>(), 0..512),
    )
        .prop_map(|(magic, form, size, rest)| {
            let mut v = magic.to_vec();
            v.extend_from_slice(&size.to_le_bytes());
            v.extend_from_slice(form);
            v.extend(rest);
            v
        })
}

// proptest's default configuration: 256 cases, more with `PROPTEST_CASES` (nightly 10k).
proptest! {
    #[test]
    fn valid_chunk_soups_walk_to_the_generators_record(s in soup()) {
        let (bytes, want) = build(&s);
        let header = iff::read_header(&mut Strict::new(&bytes), Path::new(PATH))
            .unwrap_or_else(|e| panic!("{e}"));
        let t = &header.table;
        let container = match s.kind {
            SoupKind::Riff => Container::Riff,
            SoupKind::Rf64 => Container::Rf64,
            SoupKind::Aiff => Container::Aiff,
        };
        prop_assert_eq!(t.container, container);
        prop_assert_eq!(record_of(t), want);
        prop_assert!(!t.truncated);
        let got = iff::read_all_int(Strict::new(&bytes), &header.format, Path::new(PATH))
            .unwrap_or_else(|e| panic!("{e}"));
        let want: Vec<i32> = s.samples.iter().map(|v| i32::from(*v)).collect();
        prop_assert_eq!(got, want);
    }

    #[test]
    fn arbitrary_bytes_never_panic_or_overrun(
        bytes in prop_oneof![prop::collection::vec(any::<u8>(), 0..512), container_header()]
    ) {
        exercise(&bytes);
    }

    #[test]
    fn truncated_fixtures_never_panic_or_overrun(fixture in any::<Index>(), cut in any::<Index>()) {
        let bytes = &FIXTURES[fixture.index(FIXTURES.len())];
        exercise(&bytes[..cut.index(bytes.len() + 1)]);
    }

    #[test]
    fn mutated_fixtures_never_panic_or_overrun(
        fixture in any::<Index>(),
        in_header in any::<bool>(),
        at in any::<Index>(),
        flip in 1_u8..=255,
    ) {
        let mut bytes = FIXTURES[fixture.index(FIXTURES.len())].clone();
        // Half the mutations hit the first 128 bytes, where the headers are.
        let span = if in_header { bytes.len().min(128) } else { bytes.len() };
        let i = at.index(span);
        bytes[i] ^= flip;
        exercise(&bytes);
    }
}
