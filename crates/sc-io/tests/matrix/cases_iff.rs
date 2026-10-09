//! WAV, RF64, AIFF and AIFF-C fixtures of the matrix (rules in `cases.rs`).

use super::aiff::{self, Form};
use super::cases::{FRAMES, Fixture, Symphonia, finish, spec};
use super::id3::{self, Frame, Layout, Version};
use super::parse::Container;
use super::pcm;
use super::riff::{self, Chunk, RiffLayout, WavFormat};

/// A chunk holding an `ID3v2` tag, its frames listed after it.
#[must_use]
pub fn tag_chunk(id: [u8; 4], tag: id3::Tag) -> Chunk {
    Chunk {
        id,
        payload: tag.bytes,
        nested: tag.frames,
        pad: 0,
    }
}

fn info() -> Chunk {
    riff::list_info(&[
        (b"INAM", "Matrix Tone"),
        (b"IART", "SoundCheck Ensemble"),
        (b"ISFT", "fixture builder"),
    ])
}

/// An unknown chunk of odd length (7 bytes), so a pad byte follows it.
fn odd_chunk() -> Chunk {
    Chunk::new(*b"xodd", b"odd\x00len".to_vec())
}

/// A frame count as the 32-bit field the headers hold.
#[must_use]
pub fn frames_u32(n: usize) -> u32 {
    u32::try_from(n).expect("small fixtures")
}

/// A RIFF/WAVE fixture from chunks in order.
#[must_use]
pub fn wav(name: &'static str, src: pcm::Samples, rate: u32, ch: u16, chunks: &[Chunk]) -> Fixture {
    let built = riff::build(chunks, &RiffLayout::default());
    finish(spec(name, Container::Wave, rate, ch, src), built)
}

/// WAV 16-bit stereo: `fmt `, `LIST`/`INFO`, `id3 ` (v2.3 with `frames` and `padding`),
/// `data`. The matrix uses the common frame set with Serato objects, an existing
/// `TXXX:REPLAYGAIN_TRACK_GAIN` and 512 bytes of padding.
#[must_use]
pub fn wav16_info_id3(frames: &[Frame], padding: usize) -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 16, 1);
    let chunks = [
        riff::fmt(WavFormat::Pcm, 2, 44_100, 16),
        info(),
        tag_chunk(*b"id3 ", id3::tag(Version::V23, frames, padding)),
        riff::data(&src),
    ];
    wav("wav16-info-id3v23-pad512", src, 44_100, 2, &chunks)
}

fn wav24_bwf_id3v24() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 24, 2);
    let tag = id3::tag(
        Version::V24,
        &id3::frames(Version::V24, true, Some("replaygain_track_gain")),
        0,
    );
    let chunks = [
        riff::bext(
            2,
            44_100,
            "A=PCM,F=44100,W=24,M=stereo,T=SoundCheck matrix\r\n",
        ),
        riff::fmt(WavFormat::Pcm, 2, 44_100, 24),
        riff::ixml(),
        // 300 lies inside a 441-sample head trim and clamps to 0.
        riff::cue(&[0, 300, 11_025]),
        // A loop after the trimmed head: shifted, kept.
        riff::smpl(44_100, 1_000, 11_025),
        tag_chunk(*b"ID3 ", tag),
        // Some writers pad with a space; the pad byte value is carried too.
        odd_chunk().with_pad(0x20),
        riff::data(&src),
    ];
    wav("wav24-bwf-ixml-cue-smpl-id3v24", src, 44_100, 2, &chunks)
}

fn wav24_extensible() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 24, 3);
    let chunks = [
        Chunk::new(*b"JUNK", vec![0; 28]),
        // An even-length coding history keeps the chunk free of a pad byte, so hound reads it.
        riff::bext(1, 44_100, "A=PCM,F=44100,W=24,M=stereo,T=DAW1\r\n"),
        riff::fmt(WavFormat::Extensible, 2, 44_100, 24),
        riff::fact(frames_u32(FRAMES)),
        riff::data(&src),
    ];
    wav("wav24-extensible-bext-v1-fact", src, 44_100, 2, &chunks)
}

fn wav_float32(name: &'static str, ch: u16, factor: f64, seed: u64) -> Fixture {
    let src = pcm::float_samples_scaled(44_100, ch, FRAMES, seed, factor);
    let chunks = [
        riff::fmt(WavFormat::Float, ch, 44_100, 32),
        riff::fact(frames_u32(FRAMES)),
        riff::data(&src),
    ];
    wav(name, src, 44_100, ch, &chunks)
}

fn wav16_chunks_after_data() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 16, 5);
    let tag = id3::tag(Version::V23, &id3::frames(Version::V23, true, None), 0);
    let chunks = [
        riff::fmt(WavFormat::Pcm, 2, 44_100, 16),
        riff::data(&src),
        tag_chunk(*b"id3 ", tag),
        info(),
        odd_chunk(),
    ];
    wav("wav16-chunks-after-data-odd-last", src, 44_100, 2, &chunks)
}

fn wav24_mono_odd_data() -> Fixture {
    // An odd frame count makes 24-bit mono data odd-length.
    let src = pcm::int_samples(44_100, 1, FRAMES - 1, 24, 6);
    let chunks = [riff::fmt(WavFormat::Pcm, 1, 44_100, 24), riff::data(&src)];
    let layout = RiffLayout {
        rf64_frames: None,
        omit_odd_data_pad: true,
        trailing: riff::id3v1("Matrix Tone", "SoundCheck Ensemble"),
        ..RiffLayout::default()
    };
    let built = riff::build(&chunks, &layout);
    let s = spec(
        "wav24-mono-odd-data-no-pad-id3v1",
        Container::Wave,
        44_100,
        1,
        src,
    );
    finish(s, built)
}

fn rf64_24() -> Fixture {
    let src = pcm::int_samples(48_000, 2, FRAMES, 24, 7);
    let chunks = [
        riff::fmt(WavFormat::Pcm, 2, 48_000, 24),
        riff::bext(
            2,
            48_000,
            "A=PCM,F=48000,W=24,M=stereo,T=SoundCheck matrix\r\n",
        ),
        riff::data(&src),
    ];
    let layout = RiffLayout {
        rf64_frames: Some(FRAMES as u64),
        ..RiffLayout::default()
    };
    let built = riff::build(&chunks, &layout);
    let mut s = spec("rf64-24-48k", Container::Rf64, 48_000, 2, src);
    s.symphonia = Symphonia::Refuses("symphonia 0.6.1 reads RIFF/WAVE only, not RF64");
    finish(s, built)
}

fn mono16(seed: u64) -> pcm::Samples {
    pcm::int_samples(44_100, 1, FRAMES, 16, seed)
}

fn mono_wav(name: &'static str, seed: u64, before_data: Vec<Chunk>) -> Fixture {
    let src = mono16(seed);
    let mut chunks = vec![riff::fmt(WavFormat::Pcm, 1, 44_100, 16)];
    chunks.extend(before_data);
    chunks.push(riff::data(&src));
    wav(name, src, 44_100, 1, &chunks)
}

fn two_id3_chunks() -> Fixture {
    let first = id3::tag_with(
        Layout {
            version: Version::V23,
            padding: 128,
            ext: true,
            unsync: false,
            crc: false,
        },
        &id3::frames(Version::V23, false, Some("REPLAYGAIN_TRACK_GAIN")),
    );
    let second_frames = [
        id3::text(Version::V24, *b"TIT2", "Matrix Tone (second tag)"),
        id3::txxx(Version::V24, "SOURCE", "second chunk"),
    ];
    let second = id3::tag_with(
        Layout {
            version: Version::V24,
            padding: 0,
            ext: true,
            unsync: false,
            crc: false,
        },
        &second_frames,
    );
    let chunks = vec![tag_chunk(*b"id3 ", first), tag_chunk(*b"ID3 ", second)];
    mono_wav("wav16-mono-two-id3-chunks-ext-headers", 16, chunks)
}

/// Binary data with 0xFF bytes followed by 0x00 and 0xE0, which unsynchronisation must escape.
const FF_DATA: [u8; 8] = [0x01, 0xFF, 0x00, 0x02, 0xFF, 0xE0, 0x03, 0xFF];

fn id3v24_frame_flags() -> Fixture {
    let v = Version::V24;
    let frames = [
        id3::text(v, *b"TIT2", "Matrix Tone"),
        id3::with_format_flags(id3::private("com.example.flags", &FF_DATA), 0x03),
        id3::with_format_flags(id3::txxx(v, "DLI", "data length indicator"), 0x01),
        id3::text(v, *b"TBPM", "128"),
    ];
    let tag = id3::tag(v, &frames, 64);
    let chunks = vec![tag_chunk(*b"ID3 ", tag)];
    mono_wav("wav16-mono-id3v24-frame-flags", 17, chunks)
}

fn id3v24_tag_unsync() -> Fixture {
    let v = Version::V24;
    let frames = [
        id3::text(v, *b"TIT2", "Matrix Tone"),
        id3::text(v, *b"TBPM", "128"),
        id3::private("com.example.flags", &FF_DATA),
    ]
    .map(|f| id3::with_format_flags(f, 0x02));
    let layout = Layout {
        version: v,
        padding: 32,
        ext: false,
        unsync: true,
        crc: false,
    };
    let chunks = vec![tag_chunk(*b"ID3 ", id3::tag_with(layout, &frames))];
    mono_wav("wav16-mono-id3v24-tag-unsync", 18, chunks)
}

/// A v2.4 tag whose extended header carries a CRC-32: editing would invalidate it, so the tag
/// is carried and no tags are added.
fn id3v24_ext_crc() -> Fixture {
    let v = Version::V24;
    let frames = [
        id3::text(v, *b"TIT2", "Matrix Tone"),
        id3::text(v, *b"TBPM", "128"),
        id3::txxx(v, "SOURCE", "crc-protected tag"),
    ];
    let layout = Layout {
        version: v,
        padding: 16,
        ext: true,
        unsync: false,
        crc: true,
    };
    let chunks = vec![tag_chunk(*b"ID3 ", id3::tag_with(layout, &frames))];
    mono_wav("wav16-mono-id3v24-ext-crc", 24, chunks)
}

/// The WAV fixtures.
#[must_use]
pub fn wav_fixtures() -> Vec<Fixture> {
    vec![
        wav16_info_id3(
            &id3::frames(Version::V23, true, Some("REPLAYGAIN_TRACK_GAIN")),
            512,
        ),
        wav24_bwf_id3v24(),
        wav24_extensible(),
        wav_float32("wav-float32-fact", 2, 1.0, 4),
        // Peaks near +1.9 dBFS: refused unless the gain brings them below full scale.
        wav_float32("wav-float32-mono-over-full-scale", 1, 4.0, 19),
        wav16_chunks_after_data(),
        wav24_mono_odd_data(),
        rf64_24(),
        mono_wav("wav16-mono", 14, Vec::new()),
        mono_wav(
            "wav16-mono-bext-v0",
            20,
            vec![riff::bext(0, 44_100, "A=PCM,F=44100,W=16,M=mono\r\n")],
        ),
        // A loop from sample 100 to 10,000 would lose its start to a 441-sample head trim, so
        // the trim is refused although the loop ends long after it.
        mono_wav(
            "wav16-mono-smpl-loop-in-head",
            21,
            vec![riff::smpl(44_100, 100, 10_000)],
        ),
        two_id3_chunks(),
        id3v24_frame_flags(),
        id3v24_tag_unsync(),
        id3v24_ext_crc(),
    ]
}

fn aiff(name: &'static str, form: Form, src: pcm::Samples, rate: u32, chunks: &[Chunk]) -> Fixture {
    let container = match form {
        Form::Aiff => Container::Aiff,
        Form::Aifc => Container::Aifc,
    };
    let ch = u16::try_from(src.len() / FRAMES).expect("1 or 2 channels");
    finish(
        spec(name, container, rate, ch, src),
        aiff::build(form, chunks),
    )
}

fn aiff16_serato() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 16, 8);
    let tag = id3::tag(Version::V23, &id3::frames(Version::V23, true, None), 256);
    let chunks = [
        aiff::comm(2, frames_u32(FRAMES), 16, 44_100, None),
        aiff::text(*b"NAME", "Matrix Tone"),
        aiff::text(*b"ANNO", "carried byte for byte"),
        aiff::appl(*b"MxTk", b"\x01opaque\x02"),
        aiff::ssnd(&pcm::bytes_be(&src)),
        odd_chunk(),
        tag_chunk(*b"ID3 ", tag),
    ];
    aiff("aiff16-serato-id3v23", Form::Aiff, src, 44_100, &chunks)
}

fn aiff24_48k() -> Fixture {
    let src = pcm::int_samples(48_000, 2, FRAMES, 24, 9);
    let chunks = [
        aiff::comm(2, frames_u32(FRAMES), 24, 48_000, None),
        aiff::mark(&[(1, 0, "Start"), (2, 300, "Inside cut"), (3, 12_000, "Drop")]),
        aiff::ssnd(&pcm::bytes_be(&src)),
    ];
    aiff("aiff24-48k-mark", Form::Aiff, src, 48_000, &chunks)
}

fn aiff16_ssnd_offset() -> Fixture {
    let src = mono16(22);
    let chunks = [
        aiff::comm(1, frames_u32(FRAMES), 16, 44_100, None),
        aiff::text(*b"NAME", "Offset sound data"),
        aiff::ssnd_with_offset(&pcm::bytes_be(&src), 12),
    ];
    aiff("aiff16-mono-ssnd-offset", Form::Aiff, src, 44_100, &chunks)
}

fn aifc(name: &'static str, compression: [u8; 4], label: &str, extra: bool) -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 16, 10);
    let little = &compression == b"sowt";
    let audio = if little {
        pcm::bytes_le(&src)
    } else {
        pcm::bytes_be(&src)
    };
    let mut chunks = vec![
        aiff::fver(),
        aiff::comm(
            2,
            frames_u32(FRAMES),
            16,
            44_100,
            Some((&compression, label)),
        ),
    ];
    if extra {
        chunks.extend([
            aiff::chan_stereo(),
            aiff::comt("Logic bounce"),
            aiff::inst(),
        ]);
    }
    chunks.push(aiff::ssnd(&audio));
    aiff(name, Form::Aifc, src, 44_100, &chunks)
}

/// The AIFF and AIFF-C fixtures.
#[must_use]
pub fn aiff_fixtures() -> Vec<Fixture> {
    vec![
        aiff16_serato(),
        aiff24_48k(),
        aiff16_ssnd_offset(),
        aifc(
            "aifc-none-16-chan-comt-inst",
            *b"NONE",
            "not compressed",
            true,
        ),
        aifc("aifc-sowt-16", *b"sowt", "", false),
    ]
}
