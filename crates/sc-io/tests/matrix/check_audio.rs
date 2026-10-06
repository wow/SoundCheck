//! Audio checks on a writer output: decoded PCM against the f64 reference, and the FLAC stream
//! structure (STREAMINFO, frames, MD5, SEEKTABLE, PADDING; RFC 9639).
//!
//! PCM tolerances, in LSB of the output depth, with `r = g * x * 2^(bits-1)`:
//! - exact: 0 dB on an integer source written at the same or a greater depth must equal
//!   `x << (out_bits - in_bits)` sample for sample (16 -> 24 is `x << 8`);
//! - undithered (24-bit output otherwise): every sample exactly `r` rounded half to even
//!   (`r` from the same `f64` products and pure-Rust `pow` as the writer), mean error against
//!   `r` below 0.05 LSB and RMS error at most 0.35 LSB (pure rounding gives 0.29);
//! - TPDF-dithered (16-bit output otherwise): every sample within 1.5 LSB of `r`, mean error
//!   below 0.05 LSB, RMS error within 0.4..0.6 LSB (rounding plus +/-1 LSB triangular dither
//!   gives 0.5). An undithered 16-bit result (0.29) fails, as do floor instead of round (mean
//!   -0.5) and a constant offset.

use super::apply::{ApplyArgs, gain_factor};
use super::cases::Fixture;
use super::decode::decode;
use super::expect::{out_bits, out_frames};
use super::inspect::{flac_frames, pcm_md5, seektable, streaminfo};
use super::parse::{self, Container, Kind, Parsed};
use super::pcm::Samples;

/// Decodes `out` and checks rate, channels, frame count and every sample against the
/// reference; returns the decoded samples at the output depth.
///
/// # Errors
/// The first sample or statistic that is out of tolerance.
pub fn check_pcm(fx: &Fixture, out: &[u8], args: &ApplyArgs) -> Result<Vec<i32>, String> {
    // symphonia 0.6.1 drops the last FLAC frame when an ID3v1 tag follows it, so the tag is
    // cut off first, as `sc-io` itself has to.
    let parsed = parse::parse(out)?;
    let end = match parsed.blocks.last() {
        Some(b) if parsed.container == Container::Flac && b.kind == Kind::Trailing => b.offset,
        _ => out.len(),
    };
    let decoded = decode(&out[..end], fx.ext)?;
    if decoded.sample_rate != fx.sample_rate || decoded.channels != fx.channels {
        return Err("decoded spec changed".into());
    }
    let frames = out_frames(fx, args);
    if decoded.frames() != frames {
        return Err(format!("{} frames, want {frames}", decoded.frames()));
    }
    let bits = out_bits(fx, args);
    let samples = decoded.as_bits(bits);
    let skip = usize::try_from(args.trim_samples).expect("small") * usize::from(fx.channels);
    let gain = gain_factor(args.gain_db);
    let scale = f64::from(1_u32 << (bits - 1));
    let exact = match &fx.source {
        Samples::Int {
            bits: source_bits,
            data,
        } if args.gain_db == 0.0 && bits >= *source_bits => {
            Some((&data[skip..], u32::from(bits - source_bits)))
        }
        _ => None,
    };
    let dithered = exact.is_none() && bits == 16;
    let (mut sum, mut sum_sq) = (0.0, 0.0);
    for (i, y) in samples.iter().enumerate() {
        let r = fx.source.normalised(skip + i) * gain * scale;
        let y_f = f64::from(*y);
        // Undithered rows must equal the rounded reference exactly.
        #[allow(clippy::float_cmp)]
        let bad = match exact {
            Some((data, shift)) => *y != data[i] << shift,
            None if dithered => (y_f - r).abs() > 1.5,
            None => y_f != r.round_ties_even(),
        };
        if bad {
            return Err(format!("sample {i} is {y}, reference {r:.3}"));
        }
        sum += y_f - r;
        sum_sq += (y_f - r) * (y_f - r);
    }
    // Sample counts stay far below 2^52.
    #[allow(clippy::cast_precision_loss)]
    let n = samples.len().max(1) as f64;
    let (mean, rms) = (sum / n, (sum_sq / n).sqrt());
    if mean.abs() >= 0.05 {
        return Err(format!("mean error {mean:.4} LSB"));
    }
    let rms_ok = if dithered {
        (0.4..=0.6).contains(&rms)
    } else {
        rms <= 0.35
    };
    if !rms_ok {
        return Err(format!(
            "RMS error {rms:.4} LSB (dither expected: {dithered})"
        ));
    }
    Ok(samples)
}

/// FLAC stream checks: STREAMINFO first and right (rate, channels, depth, total samples, block
/// sizes, min/max frame size from a CRC-verified frame walk, MD5 of the decoded PCM), every
/// seek point on a frame start with placeholders last, PADDING all zero, the audio as
/// [`check_pcm`] requires.
///
/// # Errors
/// The first property that is wrong.
pub fn check_flac_stream(
    fx: &Fixture,
    out: &[u8],
    parsed: &Parsed,
    args: &ApplyArgs,
) -> Result<(), String> {
    let first = parsed.blocks.iter().find(|b| b.kind == Kind::FlacBlock);
    let Some(info_block) = first.filter(|b| b.id == "STREAMINFO") else {
        return Err("STREAMINFO is not the first block".into());
    };
    let info = streaminfo(&info_block.bytes)?;
    let bits = out_bits(fx, args);
    let total = out_frames(fx, args) as u64;
    if info.sample_rate != fx.sample_rate
        || u16::from(info.channels) != fx.channels
        || info.bits != bits
        || info.total_samples != total
    {
        return Err(format!("STREAMINFO {info:?}"));
    }
    for padding in parsed.blocks.iter().filter(|b| b.id == "PADDING") {
        if padding.bytes.iter().any(|b| *b != 0) {
            return Err("PADDING holds non-zero bytes".into());
        }
    }
    let region = parsed.find(Kind::FlacFrames, "frames").ok_or("no frames")?;
    let frames = flac_frames(&region.bytes, u32::from(info.max_block))?;
    let mut next = 0_u64;
    for (i, f) in frames.iter().enumerate() {
        let last = i + 1 == frames.len();
        let max_ok = f.block_size <= u32::from(info.max_block);
        let min_ok = last || f.block_size >= u32::from(info.min_block);
        if f.first_sample != next || !max_ok || !min_ok {
            return Err(format!("frame {i}: {f:?}"));
        }
        next += u64::from(f.block_size);
    }
    let lens = frames
        .iter()
        .map(|f| u32::try_from(f.len).unwrap_or(u32::MAX));
    let (min, max) = (lens.clone().min(), lens.max());
    if next != total || min != Some(info.min_frame) || max != Some(info.max_frame) {
        return Err(format!("frames sum to {next}, sizes {min:?}..{max:?}"));
    }
    let samples = check_pcm(fx, out, args)?;
    if pcm_md5(&samples, bits) != info.md5 {
        return Err("STREAMINFO MD5 differs from the decoded PCM".into());
    }
    if let Some(table) = parsed.find(Kind::FlacBlock, "SEEKTABLE") {
        let points = seektable(&table.bytes)?;
        let real = points.iter().take_while(|p| p.sample != u64::MAX).count();
        if points[real..].iter().any(|p| p.sample != u64::MAX) {
            return Err("placeholder seek points are not last".into());
        }
        for p in &points[..real] {
            let on_frame = frames.iter().any(|f| {
                f.offset as u64 == p.offset
                    && f.first_sample == p.sample
                    && f.block_size == u32::from(p.frame_samples)
            });
            if !on_frame {
                return Err(format!("seek point {p:?} is not on a frame start"));
            }
        }
        if points[..real]
            .windows(2)
            .any(|w| w[0].sample >= w[1].sample)
        {
            return Err("seek points are not ascending".into());
        }
    }
    Ok(())
}
