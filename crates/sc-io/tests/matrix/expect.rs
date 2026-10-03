//! What a correct writer produces for patched blocks, when it must refuse, and which tag it
//! edits. Positions are parsed exactly from the layouts in the RIFF `cue `/`smpl` chunks, the
//! AIFF `MARK` chunk, the FLAC CUESHEET (RFC 9639 section 8.7), the `fact` chunk and EBU Tech
//! 3285 `bext` (`TimeReference` at byte 338, Version at 346, loudness at 412..422).

use super::apply::{ApplyArgs, TagEdit};
use super::cases::Fixture;
use super::parse::{self, Container, Kind, Parsed};
use super::riff::{BEXT_LOUDNESS, BEXT_TIME_REFERENCE, BEXT_VERSION};

fn u32_at(p: &[u8], at: usize, big: bool) -> Result<u32, String> {
    let s = parse::slice(p, at, 4)?;
    let a = [s[0], s[1], s[2], s[3]];
    Ok(if big {
        u32::from_be_bytes(a)
    } else {
        u32::from_le_bytes(a)
    })
}

fn put_u32(p: &mut [u8], at: usize, v: u32, big: bool) {
    let bytes = if big {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    };
    p[at..at + 4].copy_from_slice(&bytes);
}

/// Subtracts the trim from a 32-bit position, clamping at 0.
fn shift32(p: &mut [u8], at: usize, trim: u64, big: bool) -> Result<(), String> {
    let v = u64::from(u32_at(p, at, big)?);
    let shifted = u32::try_from(v.saturating_sub(trim)).expect("not larger than before");
    put_u32(p, at, shifted, big);
    Ok(())
}

fn u64_be(p: &[u8], at: usize) -> Result<u64, String> {
    let mut a = [0_u8; 8];
    a.copy_from_slice(parse::slice(p, at, 8)?);
    Ok(u64::from_be_bytes(a))
}

/// Output frames for `args` on `fx`.
#[must_use]
pub fn out_frames(fx: &Fixture, args: &ApplyArgs) -> usize {
    fx.frames - usize::try_from(args.trim_samples).expect("small trims")
}

/// Output bits per sample for `args` on `fx`.
#[must_use]
pub fn out_bits(fx: &Fixture, args: &ApplyArgs) -> u8 {
    args.bits
        .unwrap_or(if fx.is_float() { 24 } else { fx.bits })
}

/// The payload a correct writer emits for the patched block `id` whose input payload is `p`.
///
/// # Errors
/// When the payload is malformed or `id` has no patch rule.
pub fn patched(id: &str, p: &[u8], fx: &Fixture, args: &ApplyArgs) -> Result<Vec<u8>, String> {
    let trim = args.trim_samples;
    let mut out = p.to_vec();
    match id {
        "cue " => {
            let count = u32_at(p, 0, false)? as usize;
            for i in 0..count {
                shift32(&mut out, 4 + 24 * i + 4, trim, false)?; // dwPosition
                shift32(&mut out, 4 + 24 * i + 20, trim, false)?; // dwSampleOffset
            }
        }
        "smpl" => {
            let loops = u32_at(p, 28, false)? as usize;
            for i in 0..loops {
                shift32(&mut out, 36 + 24 * i + 8, trim, false)?; // dwStart
                shift32(&mut out, 36 + 24 * i + 12, trim, false)?; // dwEnd
            }
        }
        "MARK" => {
            let count = u16::from_be_bytes([p[0], p[1]]);
            let mut pos = 2;
            for _ in 0..count {
                shift32(&mut out, pos + 2, trim, true)?;
                let name = usize::from(*p.get(pos + 6).ok_or("short MARK")?);
                pos += 6 + (1 + name).next_multiple_of(2);
            }
        }
        "fact" => {
            let frames = u32::try_from(out_frames(fx, args)).expect("small");
            put_u32(&mut out, 0, frames, false);
        }
        "bext" => {
            let mut t = [0_u8; 8];
            t.copy_from_slice(parse::slice(p, BEXT_TIME_REFERENCE, 8)?);
            let time = u64::from_le_bytes(t) + trim;
            out[BEXT_TIME_REFERENCE..BEXT_TIME_REFERENCE + 8].copy_from_slice(&time.to_le_bytes());
            if let Some(loudness) = args.loudness {
                out[BEXT_VERSION..BEXT_VERSION + 2].copy_from_slice(&2_u16.to_le_bytes());
                out[BEXT_LOUDNESS..BEXT_LOUDNESS + 10].copy_from_slice(&loudness.bytes());
            }
        }
        "CUESHEET" => {
            let tracks = *p.get(395).ok_or("short CUESHEET")?;
            let mut pos = 396;
            for _ in 0..tracks {
                let number = *p.get(pos + 8).ok_or("short CUESHEET track")?;
                let offset = u64_be(p, pos)?;
                let new = if number == 255 || number == 170 {
                    out_frames(fx, args) as u64
                } else {
                    offset.saturating_sub(trim)
                };
                out[pos..pos + 8].copy_from_slice(&new.to_be_bytes());
                let indices = usize::from(*p.get(pos + 35).ok_or("short CUESHEET track")?);
                pos += 36 + 12 * indices;
            }
        }
        other => return Err(format!("no patch rule for {other:?}")),
    }
    Ok(out)
}

/// Why a correct writer refuses `args` on `fx`, if it must: a sampler loop that ends inside
/// the trimmed head, or a float source whose peak after gain reaches full scale (refused
/// rather than clipped).
///
/// # Panics
/// When the fixture does not parse (the matrix tests catch that first).
#[must_use]
pub fn refusal(fx: &Fixture, args: &ApplyArgs) -> Option<String> {
    let input = parse::parse(&fx.bytes).expect("fixtures parse");
    for smpl in input.blocks.iter().filter(|b| b.id == "smpl") {
        let p = &smpl.bytes;
        let loops = u32_at(p, 28, false).unwrap_or(0) as usize;
        for i in 0..loops {
            let end = u64::from(u32_at(p, 36 + 24 * i + 12, false).unwrap_or(u32::MAX));
            if args.trim_samples > 0 && end <= args.trim_samples {
                return Some(format!(
                    "sampler loop ends at {end}, inside the trimmed head"
                ));
            }
        }
    }
    let gain = 10_f64.powf(args.gain_db / 20.0);
    if fx.is_float() && fx.source.peak() * gain >= 1.0 {
        return Some(format!(
            "float peak {:.3} x gain {gain:.3} reaches full scale",
            fx.source.peak()
        ));
    }
    None
}

/// Index (in `input.blocks`) of the tag that `edits` change: the first ID3 chunk of a WAV/AIFF
/// file unless it has tag-level unsynchronisation, the Vorbis comment block of a FLAC file;
/// `None` when nothing is edited.
#[must_use]
pub fn edited_tag(input: &Parsed, edits: &[TagEdit]) -> Option<usize> {
    if edits.is_empty() {
        return None;
    }
    if input.container == Container::Flac {
        return input
            .blocks
            .iter()
            .position(|b| b.kind == Kind::FlacBlock && b.id == "VORBIS_COMMENT");
    }
    let first = input.tags.first()?;
    (first.flags & 0x80 == 0).then_some(first.chunk)
}
