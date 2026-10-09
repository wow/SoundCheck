//! Fixtures with the chunk layouts and trailing-byte faults seen in DJ libraries exported by
//! common taggers and DAWs (rules in `cases.rs`).
//!
//! AIFF: the `ID3 ` chunk last, holding `ID3v2`.3, 2.4 or (rarely) 2.2; `COMT` before `COMM` or
//! after `SSND`; `NAME`, `ANNO` and `(c) ` text chunks in front of `COMM`; a 24-bit file with
//! `CHAN`, `MARK`, `COMT` and `APPL` around `SSND`. Four faults after the last chunk, each with
//! a correct ID3 tag size: (a) two zero bytes inside the `FORM` (its size counts them), (b) two
//! zero bytes after the `FORM` end, (c) an odd last chunk without its pad byte, which the `FORM`
//! size leaves out too, (d) one stray zero byte after an even last chunk, inside the `FORM`.
//! Two neighbours of (c): the zero pad byte written but left out of the `FORM` size, alone and
//! followed by one more zero byte.
//!
//! WAV: no ID3, `LIST`/`INFO` only; `JUNK`, `fact`, `bext` and `acid` before `data`; a Pro Tools
//! set (`minf`, `elm1`, `regn`, `umid`, `DGDA`); `cue `, `bext` and a second `LIST` after
//! `data`; 48 kHz at 16 and 24 bits; the same trailing faults as in AIFF (stray bytes inside the
//! RIFF, an odd last chunk without its pad byte) and an `id3 ` chunk appended past a stale RIFF
//! size. Chunks nobody documents are filled with seeded bytes: they are only ever carried.

use super::aiff::{self, AiffLayout, Form};
use super::cases::{FRAMES, Fixture, finish, spec};
use super::cases_iff::{frames_u32, tag_chunk, wav};
use super::id3::{self, Version};
use super::parse::Container;
use super::pcm::{self, XorShift};
use super::riff::{self, Chunk, RiffLayout, WavFormat};

/// A tag with the common frame set whose total length has the requested parity (`odd`), so
/// the chunk holding it does (or does not) need a pad byte.
fn tag_of_parity(version: Version, serato: bool, padding: usize, odd: bool) -> id3::Tag {
    let frames = id3::frames(version, serato, None);
    let tag = id3::tag(version, &frames, padding);
    if (tag.bytes.len() % 2 == 1) == odd {
        tag
    } else {
        id3::tag(version, &frames, padding + 1)
    }
}

/// An `ID3v2`.2 tag (id3.org "ID3 tag version 2": three-character frame ids, 24-bit frame
/// sizes): title, artist, BPM, then 64 bytes of padding.
fn id3v22() -> Chunk {
    let mut frames = Vec::new();
    for (id, text) in [
        (b"TT2", "Matrix Tone"),
        (b"TP1", "SoundCheck Ensemble"),
        (b"TBP", "128"),
    ] {
        let len = u32::try_from(1 + text.len()).expect("short text");
        frames.extend_from_slice(id);
        frames.extend_from_slice(&len.to_be_bytes()[1..]);
        frames.push(0); // ISO-8859-1
        frames.extend_from_slice(text.as_bytes());
    }
    Chunk::new(*b"ID3 ", id3::assemble([2, 0, 0], &[], &frames, 64))
}

fn aiff_with(
    name: &'static str,
    src: pcm::Samples,
    rate: u32,
    chunks: &[Chunk],
    layout: &AiffLayout,
) -> Fixture {
    let ch = u16::try_from(src.len() / FRAMES).expect("1 or 2 channels");
    let built = aiff::build_with(Form::Aiff, chunks, layout);
    finish(spec(name, Container::Aiff, rate, ch, src), built)
}

fn stereo16(seed: u64) -> pcm::Samples {
    pcm::int_samples(44_100, 2, FRAMES, 16, seed)
}

fn comm16() -> Chunk {
    aiff::comm(2, frames_u32(FRAMES), 16, 44_100, None)
}

/// Fault (a) on the most common layout: `COMM SSND ID3` (v2.4, even), two zero bytes inside
/// the `FORM`.
fn aiff_stray2_in_form() -> Fixture {
    let src = stereo16(30);
    let chunks = [
        comm16(),
        aiff::ssnd(&pcm::bytes_be(&src)),
        tag_chunk(*b"ID3 ", tag_of_parity(Version::V24, false, 256, false)),
    ];
    let layout = AiffLayout {
        stray_inside: vec![0, 0],
        ..AiffLayout::default()
    };
    aiff_with(
        "aiff16-id3v24-2-stray-in-form",
        src,
        44_100,
        &chunks,
        &layout,
    )
}

/// Fault (b) with `COMT` before `COMM`: `COMT COMM SSND ID3` (v2.3 with Serato objects), two
/// zero bytes after the `FORM` end.
fn aiff_comt_first_2_after_form() -> Fixture {
    let src = stereo16(31);
    let chunks = [
        aiff::comt("Purchased track"),
        comm16(),
        aiff::ssnd(&pcm::bytes_be(&src)),
        tag_chunk(*b"ID3 ", tag_of_parity(Version::V23, true, 512, false)),
    ];
    let layout = AiffLayout {
        trailing: vec![0, 0],
        ..AiffLayout::default()
    };
    let name = "aiff16-comt-comm-id3v23-2-after-form";
    aiff_with(name, src, 44_100, &chunks, &layout)
}

/// Fault (c): `NAME ANNO COMM SSND ID3` (v2.4, odd) with the last pad byte left out of the file
/// and the `FORM` size.
fn aiff_odd_last_unpadded() -> Fixture {
    let src = stereo16(32);
    let chunks = [
        aiff::text(*b"NAME", "Matrix Tone"),
        aiff::text(*b"ANNO", "Exported by a tagger"),
        comm16(),
        aiff::ssnd(&pcm::bytes_be(&src)),
        tag_chunk(*b"ID3 ", tag_of_parity(Version::V24, false, 128, true)),
    ];
    let layout = AiffLayout {
        omit_last_pad: true,
        ..AiffLayout::default()
    };
    let name = "aiff16-name-anno-id3v24-odd-unpadded";
    aiff_with(name, src, 44_100, &chunks, &layout)
}

/// Fault (d): `NAME (c)  COMM SSND ID3` (v2.3, even) and one stray zero byte inside the `FORM`.
fn aiff_stray1_in_form() -> Fixture {
    let src = stereo16(33);
    let chunks = [
        aiff::text(*b"NAME", "Matrix Tone"),
        aiff::text(*b"(c) ", "2026 SoundCheck Ensemble"),
        comm16(),
        aiff::ssnd(&pcm::bytes_be(&src)),
        tag_chunk(*b"ID3 ", tag_of_parity(Version::V23, false, 64, false)),
    ];
    let layout = AiffLayout {
        stray_inside: vec![0],
        ..AiffLayout::default()
    };
    let name = "aiff16-name-copyright-id3v23-1-stray-in-form";
    aiff_with(name, src, 44_100, &chunks, &layout)
}

/// `COMM SSND COMT ID3` (v2.4): a comment chunk after the sound data.
fn aiff_comt_after_ssnd() -> Fixture {
    let src = stereo16(34);
    let chunks = [
        comm16(),
        aiff::ssnd(&pcm::bytes_be(&src)),
        aiff::comt("Comment after the sound data"),
        tag_chunk(*b"ID3 ", tag_of_parity(Version::V24, true, 0, false)),
    ];
    let layout = AiffLayout::default();
    aiff_with("aiff16-ssnd-comt-id3v24", src, 44_100, &chunks, &layout)
}

/// 24-bit `COMM CHAN MARK COMT SSND APPL ID3` (v2.4), as a DAW bounce re-tagged later.
fn aiff24_daw_chunks() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 24, 35);
    let chunks = [
        aiff::comm(2, frames_u32(FRAMES), 24, 44_100, None),
        aiff::chan_stereo(),
        // 200 lies inside a 441-sample head trim and clamps to 0.
        aiff::mark(&[(1, 200, "Intro"), (2, 16_000, "Break")]),
        aiff::comt("Bounced"),
        aiff::ssnd(&pcm::bytes_be(&src)),
        aiff::appl(*b"MxLg", &XorShift::new(36).bytes(41)),
        tag_chunk(*b"ID3 ", tag_of_parity(Version::V24, false, 32, false)),
    ];
    let layout = AiffLayout::default();
    let name = "aiff24-chan-mark-comt-appl-id3v24";
    aiff_with(name, src, 44_100, &chunks, &layout)
}

/// `COMM SSND ID3` with an `ID3v2`.2 tag: carried, never edited.
fn aiff_id3v22() -> Fixture {
    let src = stereo16(37);
    let chunks = [comm16(), aiff::ssnd(&pcm::bytes_be(&src)), id3v22()];
    let layout = AiffLayout::default();
    aiff_with("aiff16-id3v22", src, 44_100, &chunks, &layout)
}

fn info(items: &[(&[u8; 4], &str)]) -> Chunk {
    riff::list_info(items)
}

/// An `acid` chunk (Sonic Foundry ACID loop info, 24 bytes): one-shot flags, root note 60,
/// 16 beats in 4/4 at 128 BPM.
fn acid() -> Chunk {
    let mut p = 0x0000_0002_u32.to_le_bytes().to_vec(); // flags: root note set
    p.extend_from_slice(&60_u16.to_le_bytes());
    p.extend_from_slice(&0x8000_u16.to_le_bytes());
    p.extend_from_slice(&0.0_f32.to_le_bytes());
    p.extend_from_slice(&16_u32.to_le_bytes());
    p.extend_from_slice(&4_u16.to_le_bytes());
    p.extend_from_slice(&4_u16.to_le_bytes());
    p.extend_from_slice(&128.0_f32.to_le_bytes());
    Chunk::new(*b"acid", p)
}

/// `JUNK fmt fact bext acid data LIST`, 24-bit (a sample-pack export).
fn wav24_acid() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 24, 38);
    let chunks = [
        Chunk::new(*b"JUNK", vec![0; 28]),
        riff::fmt(WavFormat::Pcm, 2, 44_100, 24),
        riff::fact(frames_u32(FRAMES)),
        riff::bext(2, 44_100, "A=PCM,F=44100,W=24,M=stereo,T=loop\r\n"),
        acid(),
        riff::data(&src),
        info(&[(b"INAM", "Matrix Tone"), (b"IGNR", "House")]),
    ];
    wav("wav24-junk-fact-bext-acid-info", src, 44_100, 2, &chunks)
}

/// The Pro Tools chunk set at 48 kHz 24-bit: `JUNK bext fmt minf elm1 data regn umid DGDA
/// LIST`; the undocumented chunks hold seeded bytes (`DGDA` odd, so a pad byte follows it).
fn wav24_pro_tools() -> Fixture {
    let src = pcm::int_samples(48_000, 2, FRAMES, 24, 39);
    let mut rng = XorShift::new(40);
    let chunks = [
        Chunk::new(*b"JUNK", vec![0; 92]),
        riff::bext(1, 48_000, "A=PCM,F=48000,W=24,M=stereo,T=Session1\r\n"),
        riff::fmt(WavFormat::Pcm, 2, 48_000, 24),
        Chunk::new(*b"minf", rng.bytes(16)),
        Chunk::new(*b"elm1", rng.bytes(214)),
        riff::data(&src),
        Chunk::new(*b"regn", rng.bytes(92)),
        Chunk::new(*b"umid", rng.bytes(24)),
        Chunk::new(*b"DGDA", rng.bytes(173)),
        info(&[(b"ISFT", "Pro Tools")]),
    ];
    wav("wav24-48k-pro-tools-chunks", src, 48_000, 2, &chunks)
}

/// `fmt data cue LIST bext LIST` at 48 kHz 16-bit: cue points, `bext` and two `LIST` chunks
/// after the audio.
fn wav16_after_data() -> Fixture {
    let src = pcm::int_samples(48_000, 2, FRAMES, 16, 41);
    let chunks = [
        riff::fmt(WavFormat::Pcm, 2, 48_000, 16),
        riff::data(&src),
        // 300 lies inside a 441-sample head trim and clamps to 0.
        riff::cue(&[300, 9_600]),
        info(&[(b"INAM", "Matrix Tone"), (b"IART", "SoundCheck Ensemble")]),
        riff::bext(2, 48_000, "A=PCM,F=48000,W=16,M=stereo,T=editor\r\n"),
        info(&[(b"ICMT", "second LIST after bext")]),
    ];
    let name = "wav16-48k-cue-list-bext-list-after-data";
    wav(name, src, 48_000, 2, &chunks)
}

/// The zero pad byte of an odd last `ID3 ` chunk written after the `FORM` end, not counted in
/// its size; with `extra`, one more zero byte follows it (fault (c) with two zero bytes after
/// the `FORM`).
fn aiff_pad_after_form(name: &'static str, version: Version, extra: bool, seed: u64) -> Fixture {
    let src = stereo16(seed);
    let chunks = [
        comm16(),
        aiff::ssnd(&pcm::bytes_be(&src)),
        tag_chunk(*b"ID3 ", tag_of_parity(version, false, 96, true)),
    ];
    let layout = AiffLayout {
        pad_outside: true,
        trailing: if extra { vec![0] } else { Vec::new() },
        ..AiffLayout::default()
    };
    aiff_with(name, src, 44_100, &chunks, &layout)
}

fn wav_with(name: &'static str, seed: u64, after_data: Vec<Chunk>, layout: &RiffLayout) -> Fixture {
    let src = stereo16(seed);
    let mut chunks = vec![riff::fmt(WavFormat::Pcm, 2, 44_100, 16), riff::data(&src)];
    chunks.extend(after_data);
    let built = riff::build(&chunks, layout);
    finish(spec(name, Container::Wave, 44_100, 2, src), built)
}

/// `fmt data LIST` and two zero bytes after the last chunk inside the RIFF.
fn wav_stray_in_riff() -> Fixture {
    let layout = RiffLayout {
        stray_inside: vec![0, 0],
        ..RiffLayout::default()
    };
    let list = vec![info(&[(b"INAM", "Matrix Tone")])];
    wav_with("wav16-list-2-stray-in-riff", 42, list, &layout)
}

/// `fmt data LIST` and an odd chunk last without its pad byte, the RIFF size ending at its last
/// byte.
fn wav_odd_last_unpadded() -> Fixture {
    let layout = RiffLayout {
        omit_last_pad: true,
        ..RiffLayout::default()
    };
    let after = vec![
        info(&[(b"INAM", "Matrix Tone")]),
        Chunk::new(*b"xodd", XorShift::new(43).bytes(13)),
    ];
    wav_with("wav16-odd-last-chunk-unpadded", 44, after, &layout)
}

/// `fmt data LIST` and an `id3 ` chunk (v2.3) appended after them without updating the RIFF
/// size: the tag lies past the container end and is still edited.
fn wav_id3_past_stale_size() -> Fixture {
    let layout = RiffLayout {
        stale_last: true,
        ..RiffLayout::default()
    };
    let after = vec![
        info(&[(b"INAM", "Matrix Tone")]),
        tag_chunk(*b"id3 ", tag_of_parity(Version::V23, false, 64, true)),
    ];
    wav_with("wav16-id3-past-stale-riff-size", 45, after, &layout)
}

/// The fixtures of this module, in a fixed order.
#[must_use]
pub fn fixtures() -> Vec<Fixture> {
    vec![
        aiff_stray2_in_form(),
        aiff_comt_first_2_after_form(),
        aiff_odd_last_unpadded(),
        aiff_stray1_in_form(),
        aiff_comt_after_ssnd(),
        aiff24_daw_chunks(),
        aiff_id3v22(),
        wav24_acid(),
        wav24_pro_tools(),
        wav16_after_data(),
        aiff_pad_after_form("aiff16-id3v23-pad-after-form", Version::V23, false, 46),
        aiff_pad_after_form("aiff16-id3v24-pad-and-1-after-form", Version::V24, true, 47),
        wav_stray_in_riff(),
        wav_odd_last_unpadded(),
        wav_id3_past_stale_size(),
    ]
}
