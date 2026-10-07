//! Verifying the temp file after it is synced, before anything else happens.
//!
//! - WAV/AIFF: the header is read again (container, channels, rate, depth, frame count as the
//!   render reported them), the file length must equal the render's, and the audio bytes must
//!   hash (BLAKE3) to what the render hashed while writing them.
//! - FLAC: the render's own check (metadata as planned, STREAMINFO as written, every frame
//!   decoded by an independent decoder to the encoded samples), run on the synced file.
//!
//! Both also hash the whole file, which the journal and sidecar record and recovery compares.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use sc_core::{Error, Result};

use super::fsx::{COPY_BUFFER_BYTES, hash_file, io_err};
use crate::iff;
use crate::render::{FlacCheck, RenderReport};

/// What to check the temp file against, besides the render report.
pub(crate) enum Check {
    /// A WAV or AIFF render; `wave` says which family it must be.
    Iff {
        /// Whether the output is RIFF/WAVE (else FORM/AIFF).
        wave: bool,
    },
    /// A FLAC render's own check.
    Flac(FlacCheck),
}

/// A verification failure naming `target` (the file being processed).
fn failed(target: &Path, detail: impl Into<String>) -> Error {
    Error::VerifyFailed {
        path: target.to_path_buf(),
        detail: detail.into(),
    }
}

/// Verifies `temp` against `report` and `check`; returns the file's length and BLAKE3.
///
/// # Errors
/// [`Error::VerifyFailed`] naming `target` for any difference (a file that does not parse
/// included); [`Error::Io`]; [`Error::Cancelled`].
pub(crate) fn verify(
    temp: &Path,
    target: &Path,
    report: &RenderReport,
    check: &Check,
    cancel: &AtomicBool,
) -> Result<(u64, [u8; 32])> {
    let as_failure = |e: Error| match e {
        Error::Corrupt { detail, .. } | Error::UnsupportedFormat { detail, .. } => {
            failed(target, detail)
        }
        other => other,
    };
    let (len, hash) = match check {
        Check::Iff { wave } => verify_iff(temp, target, report, *wave).map_err(as_failure)?,
        Check::Flac(flac) => {
            flac.run(temp, cancel).map_err(as_failure)?;
            hash_file(temp)?
        }
    };
    if len != report.output_bytes {
        return Err(failed(
            target,
            format!("{len} bytes on disk, {} written", report.output_bytes),
        ));
    }
    tracing::debug!(path = %target.display(), stage = "verify", bytes = len, "verified");
    Ok((len, hash))
}

fn verify_iff(
    temp: &Path,
    target: &Path,
    report: &RenderReport,
    wave: bool,
) -> Result<(u64, [u8; 32])> {
    let mut file = File::open(temp).map_err(|e| io_err(temp, e))?;
    let header = iff::read_header(&mut file, temp)?;
    let f = &header.format;
    let container_ok = if wave {
        header.table.container == iff::Container::Riff
    } else {
        header.table.container == iff::Container::Aiff
    };
    if !container_ok
        || f.channels != report.channels
        || f.sample_rate != report.sample_rate_hz
        || f.valid_bits != report.bits_out
        || f.frames != report.frames_out
    {
        return Err(failed(target, "the header is not as written"));
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| io_err(temp, e))?;
    let mut whole = blake3::Hasher::new();
    let mut audio = blake3::Hasher::new();
    let mut buf = vec![0_u8; COPY_BUFFER_BYTES];
    let mut pos = 0_u64;
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(temp, e)),
        };
        let chunk = &buf[..n];
        whole.update(chunk);
        let end = pos + n as u64;
        let from = f.data.start.clamp(pos, end);
        let to = f.data.end.clamp(pos, end);
        if from < to {
            // Both offsets lie within this chunk, so they fit in usize.
            let a = usize::try_from(from - pos).unwrap_or(0);
            let b = usize::try_from(to - pos).unwrap_or(0);
            audio.update(&chunk[a..b]);
        }
        pos = end;
    }
    if *audio.finalize().as_bytes() != report.pcm_hash {
        return Err(failed(
            target,
            "the audio on disk differs from the audio written",
        ));
    }
    Ok((pos, *whole.finalize().as_bytes()))
}
