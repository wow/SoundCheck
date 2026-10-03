//! The checks every writer output must pass: the block sequence against the fixture's
//! expectations (carried blocks byte-identical with their pad byte value, patched blocks equal
//! to the bytes `expect.rs` computes, replaced blocks present at their place, dropped blocks
//! absent, nothing else), the edited tag (`check_tag`), DJ-safe headers, and the audio
//! (`check_audio.rs`).

use super::apply::{Applied, ApplyArgs, TagEdit};
use super::cases::{Expect, Fixture};
use super::check_audio::{check_flac_stream, check_pcm};
use super::expect::{edited_tag, out_bits, out_frames, patched};
use super::inspect;
use super::parse::{self, Block, Container, Kind, Parsed, frame_text, vorbis_value};

/// Every check on one output: blocks, tags, the writer's report, headers, audio.
///
/// # Errors
/// The first failing check, prefixed with the fixture name.
pub fn check_output(
    fx: &Fixture,
    out: &[u8],
    args: &ApplyArgs,
    edits: &[TagEdit],
    applied: Applied,
) -> Result<(), String> {
    let run = || -> Result<(), String> {
        let parsed = parse::parse(out)?;
        check_blocks(fx, &parsed, args, edits)?;
        let input = parse::parse(&fx.bytes)?;
        let want = edited_tag(&input, edits).is_some();
        if applied.tags_added != want {
            return Err(format!(
                "reported tags_added {}, want {want}",
                applied.tags_added
            ));
        }
        if fx.is_iff() {
            check_iff_header(fx, &parsed, args)?;
            check_pcm(fx, out, args).map(drop)
        } else {
            check_flac_stream(fx, out, &parsed, args)
        }
    };
    run().map_err(|e| format!("{} {args:?}: {e}", fx.name))
}

fn first_difference(a: &[u8], b: &[u8]) -> String {
    let at = a.iter().zip(b).position(|(x, y)| x != y);
    match at {
        Some(i) => format!("byte {i}: {:#04x} instead of {:#04x}", a[i], b[i]),
        None => format!("length {} instead of {}", a.len(), b.len()),
    }
}

/// Checks the block sequence of `out` against the fixture's expectations; the tag that
/// `edits` change is checked by [`check_tag`] instead of item by item.
///
/// # Errors
/// The first block that breaks the rules.
pub fn check_blocks(
    fx: &Fixture,
    out: &Parsed,
    args: &ApplyArgs,
    edits: &[TagEdit],
) -> Result<(), String> {
    let input = parse::parse(&fx.bytes)?;
    if input.blocks.len() != fx.expected.len() {
        return Err("the fixture does not parse to its listing".into());
    }
    let edited = edited_tag(&input, edits);
    let (mut i, mut j) = (0, 0);
    while i < input.blocks.len() {
        let (want, expect) = (&input.blocks[i], &fx.expected[i].expect);
        if matches!(expect, Expect::Dropped(_)) {
            i += 1;
            continue;
        }
        let got = out
            .blocks
            .get(j)
            .ok_or_else(|| format!("missing from the output: {:?} {:?}", want.kind, want.id))?;
        if got.kind != want.kind || got.id != want.id {
            return Err(format!(
                "output block {j} is {:?} {:?}, expected {:?} {:?}",
                got.kind, got.id, want.kind, want.id
            ));
        }
        let rewritten = edited == Some(i);
        match expect {
            Expect::Carried if !rewritten => {
                if got.bytes != want.bytes || got.pad != want.pad {
                    let diff = first_difference(&got.bytes, &want.bytes);
                    return Err(format!("{:?} {:?} not carried: {diff}", want.kind, want.id));
                }
            }
            Expect::Patched { fields } => {
                let bytes = patched(&want.id, &want.bytes, fx, args)?;
                if got.bytes != bytes || got.pad != want.pad {
                    let diff = first_difference(&got.bytes, &bytes);
                    return Err(format!("{:?} patch of {fields:?} wrong: {diff}", want.id));
                }
            }
            _ => {}
        }
        let carried_pad = matches!(expect, Expect::Carried | Expect::Patched { .. }) && !rewritten;
        if got.kind == Kind::Chunk && got.bytes.len() % 2 == 1 {
            match got.pad {
                None => return Err(format!("chunk {:?} lacks its pad byte", got.id)),
                Some(v) if v != 0 && !carried_pad => {
                    return Err(format!("chunk {:?} has pad byte {v:#04x}, not 0", got.id));
                }
                _ => {}
            }
        }
        if rewritten {
            check_tag(&input, i, out, j, edits)?;
            i += 1 + input.items_after(i).len();
            j += 1 + out.items_after(j).len();
        } else {
            i += 1;
            j += 1;
        }
    }
    match out.blocks.get(j) {
        Some(extra) => Err(format!(
            "unexpected output block {:?} {:?}",
            extra.kind, extra.id
        )),
        None => Ok(()),
    }
}

fn split_head(items: &[Block]) -> (&[Block], &[Block]) {
    let n = items
        .iter()
        .take_while(|b| matches!(b.kind, Kind::Id3ExtHeader | Kind::VorbisVendor))
        .count();
    items.split_at(n)
}

/// Checks one of SoundCheck's items: value, and for ID3 a permitted text encoding (0/1 in
/// v2.3, 0..=3 in v2.4) and frame flags 0.
fn check_ours(o: &Block, edit: &TagEdit, version: Option<u8>) -> Result<(), String> {
    let value = match version {
        None => vorbis_value(o),
        Some(v) => {
            if o.bytes[8..10] != [0, 0] {
                return Err(format!(
                    "{} has frame flags {:?}",
                    edit.label,
                    &o.bytes[8..10]
                ));
            }
            let (enc, value) =
                frame_text(v, o).ok_or_else(|| format!("{} is not a text frame", edit.label))?;
            let allowed = if v == 3 { enc <= 1 } else { enc <= 3 };
            if !allowed {
                return Err(format!("{} uses text encoding {enc} in v2.{v}", edit.label));
            }
            value
        }
    };
    if value == edit.value {
        Ok(())
    } else {
        Err(format!(
            "{} is {value:?}, want {:?}",
            edit.label, edit.value
        ))
    }
}

/// Checks the edited tag after block `i` of `input` / block `j` of `out`: header items carried
/// (a v2.3 extended header's padding-size field updated), every item whose label an edit names
/// replaced in place, every other item identical and in order, the remaining edits appended
/// once in order; for ID3 the version, revision and flags unchanged and the padding used
/// rather than the tag grown when it fits.
///
/// # Errors
/// The first difference.
pub fn check_tag(
    input: &Parsed,
    i: usize,
    out: &Parsed,
    j: usize,
    edits: &[TagEdit],
) -> Result<(), String> {
    let vorbis = input.container == Container::Flac;
    let same = |a: &str, b: &str| {
        if vorbis {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let tags = |p: &Parsed, chunk: usize| p.tags.iter().find(|t| t.chunk == chunk).cloned();
    let (tin, tout) = if vorbis {
        (None, None)
    } else {
        let tin = tags(input, i).ok_or("no input tag")?;
        let tout = tags(out, j).ok_or("no output tag")?;
        if (tin.version, tin.revision, tin.flags) != (tout.version, tout.revision, tout.flags) {
            return Err(format!("tag header changed: {tin:?} -> {tout:?}"));
        }
        (Some(tin), Some(tout))
    };
    let version = tin.as_ref().map(|t| t.version);
    let (in_head, ins) = split_head(input.items_after(i));
    let (out_head, outs) = split_head(out.items_after(j));
    if in_head.len() != out_head.len() {
        return Err("extended header or vendor added or removed".into());
    }
    for (a, b) in in_head.iter().zip(out_head) {
        let (mut a_bytes, mut b_bytes) = (a.bytes.clone(), b.bytes.clone());
        if let (Some(3), Some(tout)) = (version, &tout) {
            // v2.3 extended header: bytes 6..10 hold the padding size.
            let pad = u32::try_from(tout.padding).expect("small").to_be_bytes();
            if b_bytes.get(6..10) != Some(&pad[..]) {
                return Err("v2.3 extended header padding size is stale".into());
            }
            a_bytes.truncate(6);
            b_bytes.truncate(6);
        }
        if a_bytes != b_bytes {
            return Err(format!("{:?} changed", a.id));
        }
    }
    let appended: Vec<&TagEdit> = edits
        .iter()
        .filter(|e| !ins.iter().any(|b| same(&b.id, e.label)))
        .collect();
    if outs.len() != ins.len() + appended.len() {
        return Err(format!(
            "{} items, want {} carried or replaced and {} appended",
            outs.len(),
            ins.len(),
            appended.len()
        ));
    }
    for (a, o) in ins.iter().zip(outs) {
        match edits.iter().find(|e| same(&a.id, e.label)) {
            Some(edit) if same(&o.id, &a.id) => check_ours(o, edit, version)?,
            Some(edit) => return Err(format!("{} not replaced in place", edit.label)),
            None if o.listed() == a.listed() => {}
            None => return Err(format!("item {:?} not carried", a.id)),
        }
    }
    for (edit, o) in appended.iter().zip(&outs[ins.len()..]) {
        if o.id != edit.label {
            return Err(format!("appended {:?}, want {}", o.id, edit.label));
        }
        check_ours(o, edit, version)?;
    }
    if let (Some(tin), Some(tout)) = (tin, tout) {
        let bytes = |items: &[Block]| items.iter().map(|b| b.bytes.len()).sum::<usize>();
        let grown = bytes(outs).saturating_sub(bytes(ins));
        if tin.padding >= grown && tout.size != tin.size {
            return Err(format!(
                "tag grew from {} to {} bytes although {} bytes of padding could hold {grown}",
                tin.size, tout.size, tin.padding
            ));
        }
    }
    Ok(())
}

/// DJ-safe header checks: WAV outputs are RIFF/WAVE with a 16-byte `fmt ` of format 0x0001;
/// AIFF outputs are FORM/AIFF with an 18-byte `COMM`; `SSND` offset and block size 0; rate,
/// channels, depth and frame count as expected.
///
/// # Errors
/// The first header field that is wrong.
pub fn check_iff_header(fx: &Fixture, out: &Parsed, args: &ApplyArgs) -> Result<(), String> {
    let bits = out_bits(fx, args);
    let frames = out_frames(fx, args);
    let align = usize::from(fx.channels) * usize::from(bits / 8);
    let chunk = |id: &str| {
        out.find(Kind::Chunk, id)
            .ok_or_else(|| format!("no {id:?} chunk"))
    };
    match fx.container {
        Container::Wave | Container::Rf64 => {
            if out.container != Container::Wave {
                return Err("output is not RIFF/WAVE".into());
            }
            let fmt = inspect::wav_fmt(&chunk("fmt ")?.bytes)?;
            let want = inspect::WavFmt {
                len: 16,
                format_tag: 1,
                channels: fx.channels,
                sample_rate: fx.sample_rate,
                byte_rate: fx.sample_rate * u32::try_from(align).expect("small"),
                block_align: u16::try_from(align).expect("small"),
                bits: u16::from(bits),
                sub_format: None,
            };
            if fmt != want {
                return Err(format!("fmt {fmt:?}, want {want:?}"));
            }
            if chunk("data")?.bytes.len() != frames * align {
                return Err("data length does not match the frame count".into());
            }
        }
        Container::Aiff | Container::Aifc => {
            if out.container != Container::Aiff {
                return Err("output is not FORM/AIFF".into());
            }
            let comm = inspect::aiff_comm(&chunk("COMM")?.bytes, false)?;
            let ok = comm.len == 18
                && comm.channels == fx.channels
                && usize::try_from(comm.frames).ok() == Some(frames)
                && comm.bits == u16::from(bits)
                && comm.sample_rate.to_bits() == f64::from(fx.sample_rate).to_bits();
            if !ok {
                return Err(format!("COMM {comm:?}"));
            }
            let ssnd = &chunk("SSND")?.bytes;
            if ssnd.len() < 8 || ssnd[..8] != [0; 8] || ssnd.len() != 8 + frames * align {
                return Err("SSND offset/block size not 0 or length wrong".into());
            }
        }
        Container::Flac => return Err("not an IFF fixture".into()),
    }
    Ok(())
}
