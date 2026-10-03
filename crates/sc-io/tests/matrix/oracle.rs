//! A reference writer that produces what the matrix expects, built from the fixture builders.
//! It proves that the checks accept a correct output for every fixture and row; its options
//! inject one specific mistake each, which the checks must reject. It is not the product
//! writer and never runs on user files.

use super::aiff::{self, Form};
use super::apply::{Applied, ApplyArgs, TagEdit};
use super::cases::{Expect, Fixture};
use super::expect::{edited_tag, out_bits, out_frames, patched, refusal};
use super::flac;
use super::id3::{self, Version};
use super::inspect::seektable;
use super::parse::{self, Block, Container, Kind, Parsed};
use super::pcm::{self, Samples, XorShift};
use super::riff::{self, Chunk, RiffLayout, WavFormat};

/// One injected mistake (all off by default).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Output these samples instead of the ideal ones.
    pub samples: Option<Vec<i32>>,
    /// Text encoding byte for SoundCheck's ID3 frames instead of the version's default.
    pub text_encoding: Option<u8>,
    /// Frame flags on SoundCheck's ID3 frames.
    pub frame_flags: [u8; 2],
    /// Append SoundCheck's items even when one with the same label exists.
    pub append_existing: bool,
    /// Carry this patched block unchanged.
    pub unpatched: Option<&'static str>,
    /// Grow the tag even when its padding could hold the new frames.
    pub grow_tag: bool,
}

/// The ideal output samples: exact shifts where the reference is exact, TPDF dither (seeded,
/// +/-1 LSB triangular) at 16-bit otherwise, plain rounding at 24-bit.
#[must_use]
pub fn ideal_samples(fx: &Fixture, args: &ApplyArgs) -> Vec<i32> {
    let bits = out_bits(fx, args);
    let skip = usize::try_from(args.trim_samples).expect("small") * usize::from(fx.channels);
    let gain = 10_f64.powf(args.gain_db / 20.0);
    let scale = f64::from(1_u32 << (bits - 1));
    if let Samples::Int { bits: sb, data } = &fx.source
        && args.gain_db == 0.0
        && bits >= *sb
    {
        return data[skip..].iter().map(|x| x << (bits - sb)).collect();
    }
    let mut rng = XorShift::new(0x00D1_7E12);
    (skip..fx.source.len())
        .map(|i| {
            let mut r = fx.source.normalised(i) * gain * scale;
            if bits == 16 {
                r += f64::midpoint(rng.next_bipolar(), rng.next_bipolar());
            }
            // Clamped to the output range, so the cast is exact.
            #[allow(clippy::cast_possible_truncation)]
            let y = r.round().clamp(-scale, scale - 1.0) as i32;
            y
        })
        .collect()
}

/// Writes the expected output of `args` and `edits` on `fx`.
///
/// # Errors
/// The refusal a correct writer gives.
pub fn write(
    fx: &Fixture,
    args: &ApplyArgs,
    edits: &[TagEdit],
    opts: &Options,
) -> Result<(Vec<u8>, Applied), String> {
    if let Some(why) = refusal(fx, args) {
        return Err(why);
    }
    let input = parse::parse(&fx.bytes)?;
    let samples = opts
        .samples
        .clone()
        .unwrap_or_else(|| ideal_samples(fx, args));
    let edited = edited_tag(&input, edits);
    let bytes = if fx.is_iff() {
        write_iff(fx, args, &input, edited, edits, &samples, opts)?
    } else {
        write_flac(fx, args, &input, edited, edits, &samples, opts)?
    };
    let applied = Applied {
        tags_added: edited.is_some(),
    };
    Ok((bytes, applied))
}

fn fourcc(id: &str) -> [u8; 4] {
    id.as_bytes().try_into().expect("four-character ids")
}

fn write_iff(
    fx: &Fixture,
    args: &ApplyArgs,
    input: &Parsed,
    edited: Option<usize>,
    edits: &[TagEdit],
    samples: &[i32],
    opts: &Options,
) -> Result<Vec<u8>, String> {
    let bits = out_bits(fx, args);
    let frames = u32::try_from(out_frames(fx, args)).expect("small");
    let out = Samples::Int {
        bits,
        data: samples.to_vec(),
    };
    let mut chunks = Vec::new();
    let mut trailing = Vec::new();
    for (i, (b, e)) in input.blocks.iter().zip(&fx.expected).enumerate() {
        if b.kind == Kind::Trailing {
            trailing.clone_from(&b.bytes);
        }
        if b.kind != Kind::Chunk {
            continue;
        }
        let id = fourcc(&b.id);
        let carried = Chunk::new(id, b.bytes.clone()).with_pad(b.pad.unwrap_or(0));
        let chunk = match (&e.expect, b.id.as_str()) {
            (Expect::Dropped(_), _) => continue,
            (Expect::Replaced, "fmt ") => {
                riff::fmt(WavFormat::Pcm, fx.channels, fx.sample_rate, u16::from(bits))
            }
            (Expect::Replaced, "data") => riff::data(&out),
            (Expect::Replaced, "COMM") => {
                aiff::comm(fx.channels, frames, u16::from(bits), fx.sample_rate, None)
            }
            (Expect::Replaced, "SSND") => aiff::ssnd(&pcm::bytes_be(&out)),
            (Expect::Patched { .. }, id) if opts.unpatched == Some(id) => carried,
            (Expect::Patched { .. }, _) => {
                Chunk::new(id, patched(&b.id, &b.bytes, fx, args)?).with_pad(b.pad.unwrap_or(0))
            }
            _ if edited == Some(i) => Chunk::new(id, edit_id3(input, i, edits, opts)?),
            _ => carried,
        };
        chunks.push(chunk);
    }
    Ok(match input.container {
        Container::Wave | Container::Rf64 => {
            let layout = RiffLayout {
                trailing,
                ..RiffLayout::default()
            };
            riff::build(&chunks, &layout).bytes
        }
        _ => {
            let mut bytes = aiff::build(Form::Aiff, &chunks).bytes;
            bytes.extend(trailing);
            bytes
        }
    })
}

/// SoundCheck's frame for `edit` in a tag of `version`.
fn our_frame(version: Version, edit: &TagEdit, opts: &Options) -> id3::Frame {
    let mut frame = match edit.label.split_once(':') {
        Some(("TXXX", desc)) => id3::txxx(version, desc, &edit.value),
        _ => id3::text(version, fourcc(edit.label), &edit.value),
    };
    if let Some(enc) = opts.text_encoding {
        frame.body[0] = enc;
    }
    frame.flags = opts.frame_flags;
    frame
}

fn edit_id3(
    input: &Parsed,
    i: usize,
    edits: &[TagEdit],
    opts: &Options,
) -> Result<Vec<u8>, String> {
    let tag = input.tags.iter().find(|t| t.chunk == i).ok_or("no tag")?;
    let version = if tag.version == 3 {
        Version::V23
    } else {
        Version::V24
    };
    let items = input.items_after(i);
    let (head, frames): (Vec<&Block>, Vec<&Block>) =
        items.iter().partition(|b| b.kind == Kind::Id3ExtHeader);
    let mut body = Vec::new();
    for f in &frames {
        match edits.iter().find(|e| e.label == f.id) {
            Some(edit) if !opts.append_existing => {
                body.extend(id3::frame_bytes(version, &our_frame(version, edit, opts)));
            }
            _ => body.extend_from_slice(&f.bytes),
        }
    }
    for edit in edits {
        if opts.append_existing || !frames.iter().any(|f| f.id == edit.label) {
            body.extend(id3::frame_bytes(version, &our_frame(version, edit, opts)));
        }
    }
    let old: usize = frames.iter().map(|f| f.bytes.len()).sum();
    let room = old + tag.padding;
    let padding = if body.len() <= room && !opts.grow_tag {
        room - body.len()
    } else {
        tag.padding
    };
    let mut ext = head.first().map(|b| b.bytes.clone()).unwrap_or_default();
    if version == Version::V23 && ext.len() >= 10 {
        ext[6..10].copy_from_slice(&u32::try_from(padding).expect("small").to_be_bytes());
    }
    let raw = &input.blocks[i].bytes;
    Ok(id3::assemble(
        [raw[3], raw[4], raw[5]],
        &ext,
        &body,
        padding,
    ))
}

fn vorbis_edit(vendor: &[u8], fields: &[&Block], edits: &[TagEdit], opts: &Options) -> Vec<u8> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let mut out: Vec<String> = Vec::new();
    for f in fields {
        let text = String::from_utf8_lossy(&f.bytes).into_owned();
        match edits.iter().find(|e| same(&f.id, e.label)) {
            Some(edit) if !opts.append_existing => out.push(format!("{}={}", f.id, edit.value)),
            _ => out.push(text),
        }
    }
    for edit in edits {
        if opts.append_existing || !fields.iter().any(|f| same(&f.id, edit.label)) {
            out.push(format!("{}={}", edit.label, edit.value));
        }
    }
    flac::vorbis(&String::from_utf8_lossy(vendor), &out).0
}

fn write_flac(
    fx: &Fixture,
    args: &ApplyArgs,
    input: &Parsed,
    edited: Option<usize>,
    edits: &[TagEdit],
    samples: &[i32],
    opts: &Options,
) -> Result<Vec<u8>, String> {
    let bits = out_bits(fx, args);
    let enc = flac::encode(samples, bits, fx.channels, fx.sample_rate);
    let blob = |kind: Kind| {
        input
            .blocks
            .iter()
            .find(|b| b.kind == kind)
            .map(|b| b.bytes.clone())
            .unwrap_or_default()
    };
    let mut blocks: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut growth = 0_usize;
    for (i, b) in input.blocks.iter().enumerate() {
        if b.kind != Kind::FlacBlock {
            continue;
        }
        let (ty, payload) = match b.id.as_str() {
            "STREAMINFO" => (
                0,
                flac::streaminfo(&enc, samples, bits, fx.channels, fx.sample_rate),
            ),
            "PADDING" => (1, b.bytes.clone()),
            "SEEKTABLE" => {
                let points = seektable(&b.bytes)?;
                let real = points
                    .iter()
                    .filter(|p| p.sample != u64::MAX)
                    .count()
                    .max(1);
                let every = enc.sizes.len().div_ceil(real).max(1);
                (
                    3,
                    flac::seektable(&enc, every, points.len() - real.min(points.len())),
                )
            }
            "VORBIS_COMMENT" if edited == Some(i) => {
                let items = input.items_after(i);
                let fields: Vec<&Block> = items.iter().skip(1).collect();
                let p = vorbis_edit(&items[0].bytes, &fields, edits, opts);
                growth += p.len().saturating_sub(b.bytes.len());
                (4, p)
            }
            "CUESHEET" if opts.unpatched == Some("CUESHEET") => (5, b.bytes.clone()),
            "CUESHEET" => (5, patched(&b.id, &b.bytes, fx, args)?),
            id if id.starts_with("APPLICATION") => (2, b.bytes.clone()),
            "VORBIS_COMMENT" => (4, b.bytes.clone()),
            "PICTURE" => (6, b.bytes.clone()),
            other => {
                let ty = other
                    .trim_start_matches("TYPE-")
                    .parse()
                    .map_err(|_| other.to_string())?;
                (ty, b.bytes.clone())
            }
        };
        blocks.push((ty, payload));
    }
    for (ty, payload) in &mut blocks {
        if *ty == 1 {
            let len = payload.len().saturating_sub(growth);
            *payload = vec![0; len];
            growth = 0;
        }
    }
    Ok(flac::assemble(
        &blob(Kind::LeadingTag),
        &blocks,
        &enc.audio,
        &blob(Kind::Trailing),
    ))
}
