//! What a correct writer produces for patched blocks, when it must refuse, and which tag it
//! edits. Positions are parsed exactly from the layouts in the RIFF `cue `/`smpl` chunks, the
//! AIFF `MARK` chunk, the FLAC CUESHEET (RFC 9639 section 8.7), the `fact` chunk and EBU Tech
//! 3285 `bext` (`TimeReference` at byte 338, Version at 346, loudness at 412..422).

use super::apply::{ApplyArgs, BextLoudness, Refusal, TagEdit, gain_factor};
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
            // New loudness when it is given (any version is upgraded to 2); after a gain without
            // it the old values of a version 2 chunk would be stale, so all five become "not
            // measured" (7FFFh); versions 0 and 1 hold no loudness and are left alone.
            let version = u16::from_le_bytes([p[BEXT_VERSION], p[BEXT_VERSION + 1]]);
            let loudness =
                args.loudness.or(
                    (args.gain_db != 0.0 && version >= 2).then_some(BextLoudness {
                        value: BextLoudness::UNMEASURED,
                        range: BextLoudness::UNMEASURED,
                        max_true_peak: BextLoudness::UNMEASURED,
                        max_momentary: BextLoudness::UNMEASURED,
                        max_short_term: BextLoudness::UNMEASURED,
                    }),
                );
            if let Some(loudness) = loudness {
                out[BEXT_VERSION..BEXT_VERSION + 2].copy_from_slice(&2_u16.to_le_bytes());
                out[BEXT_LOUDNESS..BEXT_LOUDNESS + 10].copy_from_slice(&loudness.bytes());
            }
        }
        "CUESHEET" => patch_cuesheet(p, &mut out, trim, out_frames(fx, args) as u64)?,
        other => return Err(format!("no patch rule for {other:?}")),
    }
    Ok(out)
}

/// CUESHEET under a head trim (RFC 9639 section 8.7): every index point's absolute position
/// (track offset + index offset) moves by the trim and clamps at 0; each track's offset becomes
/// its first index's new absolute position and its index offsets are rebased on it, so a pregap
/// shrinks instead of the track start sliding; the lead-out (track 170 or 255) is the new total.
/// A track without index points keeps its offset minus the trim (clamped).
fn patch_cuesheet(p: &[u8], out: &mut [u8], trim: u64, total: u64) -> Result<(), String> {
    let tracks = *p.get(395).ok_or("short CUESHEET")?;
    let mut pos = 396;
    for _ in 0..tracks {
        let number = *p.get(pos + 8).ok_or("short CUESHEET track")?;
        let offset = u64_be(p, pos)?;
        let count = usize::from(*p.get(pos + 35).ok_or("short CUESHEET track")?);
        let mut absolute = Vec::with_capacity(count);
        for k in 0..count {
            let index = u64_be(p, pos + 36 + 12 * k)?;
            absolute.push((offset + index).saturating_sub(trim));
        }
        let new_offset = if number == 255 || number == 170 {
            total
        } else {
            absolute
                .first()
                .copied()
                .unwrap_or(offset.saturating_sub(trim))
        };
        out[pos..pos + 8].copy_from_slice(&new_offset.to_be_bytes());
        for (k, abs) in absolute.iter().enumerate() {
            let at = pos + 36 + 12 * k;
            out[at..at + 8].copy_from_slice(&(abs - new_offset).to_be_bytes());
        }
        pos += 36 + 12 * count;
    }
    Ok(())
}

/// Why a correct writer refuses `args` on `fx`, if it must, and with which error class: a
/// head trim that starts inside a sampler loop (`invalidArgument`: the loop would be cut or
/// shortened), or a float source with a sample at or above +1.0 or below -1.0 of full scale
/// after gain (`wouldClip`: refused rather than clipped; -1.0 itself is a valid code).
///
/// # Panics
/// When the fixture does not parse (the matrix tests catch that first).
#[must_use]
pub fn refusal(fx: &Fixture, args: &ApplyArgs) -> Option<Refusal> {
    let input = parse::parse(&fx.bytes).expect("fixtures parse");
    let trim = args.trim_samples;
    for smpl in input.blocks.iter().filter(|b| b.id == "smpl") {
        let p = &smpl.bytes;
        let loops = u32_at(p, 28, false).unwrap_or(0) as usize;
        for i in 0..loops {
            let start = u64::from(u32_at(p, 36 + 24 * i + 8, false).unwrap_or(0));
            let end = u64::from(u32_at(p, 36 + 24 * i + 12, false).unwrap_or(0));
            if trim > 0 && (start < trim || end <= trim) {
                return Some(Refusal::new(
                    "invalidArgument",
                    format!("the trim of {trim} cuts the sampler loop {start}..{end}"),
                ));
            }
        }
    }
    let gain = gain_factor(args.gain_db);
    let skip = usize::try_from(trim).expect("small") * usize::from(fx.channels);
    if fx.is_float() {
        let (max, min) = (skip..fx.source.len())
            .map(|i| fx.source.normalised(i))
            .fold((0.0_f64, 0.0_f64), |(hi, lo), x| (hi.max(x), lo.min(x)));
        if max * gain >= 1.0 || min * gain < -1.0 {
            return Some(Refusal::new(
                "wouldClip",
                format!(
                    "float peaks {max:.3} / {min:.3} x gain {gain:.3} leave -1.0..1.0 of full scale"
                ),
            ));
        }
    }
    None
}

/// Why requested tag edits are not written, if they are not: a WAV/AIFF file with more than
/// one ID3 chunk (readers disagree on which one counts, so both are carried), a tag with
/// tag-level unsynchronisation, or an extended header with a CRC (both carried unchanged).
#[must_use]
pub fn tags_not_added(input: &Parsed) -> Option<&'static str> {
    if input.container == Container::Flac {
        return None;
    }
    let first = input.tags.first()?;
    if input.tags.len() > 1 {
        return Some("two ID3 tags");
    }
    if first.flags & 0x80 != 0 {
        return Some("tag-level unsynchronisation");
    }
    let ext = input
        .items_after(first.chunk)
        .first()
        .filter(|b| b.kind == Kind::Id3ExtHeader);
    let crc = ext.is_some_and(|b| {
        if first.version == 3 {
            b.bytes.get(4).is_some_and(|f| f & 0x80 != 0)
        } else {
            b.bytes.get(5).is_some_and(|f| f & 0x20 != 0)
        }
    });
    crc.then_some("extended header with a CRC")
}

/// Index (in `input.blocks`) of the tag that `edits` change: the ID3 chunk of a WAV/AIFF file
/// unless [`tags_not_added`] gives a reason, the Vorbis comment block of a FLAC file; `None`
/// when nothing is edited.
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
    if tags_not_added(input).is_some() {
        return None;
    }
    input.tags.first().map(|t| t.chunk)
}

/// Whether two tag item labels name the same item: Vorbis field names and the descriptions of
/// ID3 `TXXX` frames compare case-insensitively, other ID3 labels exactly.
#[must_use]
pub fn same_label(vorbis: bool, a: &str, b: &str) -> bool {
    if vorbis || (a.starts_with("TXXX:") && b.starts_with("TXXX:")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}
