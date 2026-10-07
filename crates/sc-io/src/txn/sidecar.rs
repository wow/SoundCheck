//! The sidecar `<file>.soundcheck.json` written next to every processed file: what was asked,
//! what the render did, the hashes of the original and the output, and where the backup is.
//!
//! Keys come in a fixed order and every value derives from the input, the request and the
//! version, except `transaction` and `processed_at`, which say which run wrote it. The file is
//! replaced atomically (temp file, sync, rename), so a reader never sees half of one.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use sc_core::{Error, RenderRequest, Result};

use super::fsx::{hex, io_err, remove_if_exists, sync_dir, sync_file, temp_name};
use super::journal::{Entry, TxnKind};
use crate::render::{BlockFate, RenderReport};

/// Version of the sidecar layout; a change that breaks readers increments it.
pub const SIDECAR_SCHEMA: u32 = 1;

/// What every sidecar's name ends with.
pub const SIDECAR_SUFFIX: &str = ".soundcheck.json";

/// How many source blocks met each fate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockCounts {
    /// Copied byte for byte.
    pub carried: usize,
    /// Copied with positions or loudness fields rewritten.
    pub patched: usize,
    /// Tags with SoundCheck's items written.
    pub edited: usize,
    /// Written anew.
    pub replaced: usize,
    /// Left out.
    pub dropped: usize,
}

/// What the render did, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderSummary {
    /// Frames in the source.
    pub frames_in: u64,
    /// Frames written.
    pub frames_out: u64,
    /// Sample rate, Hz.
    pub sample_rate_hz: u32,
    /// Channels.
    pub channels: u16,
    /// Output bits per sample.
    pub bits_out: u16,
    /// Whether the samples are an exact copy or shift of the source's.
    pub exact: bool,
    /// Whether TPDF dither was added.
    pub dithered: bool,
    /// Samples moved to the largest or smallest code by rounding or dither.
    pub samples_saturated: u64,
    /// BLAKE3 (hex) of the audio as written.
    pub pcm_blake3: String,
    /// Source blocks by fate.
    pub blocks: BlockCounts,
    /// Whether the requested tag edits were written.
    pub tags_added: bool,
    /// Why requested tag edits were not written.
    pub tags_not_added: Option<String>,
    /// Loudness tags the render left stale.
    pub stale_loudness_tags: Vec<String>,
}

impl RenderSummary {
    /// The summary of `report`.
    #[must_use]
    pub fn of(report: &RenderReport) -> Self {
        Self {
            frames_in: report.frames_in,
            frames_out: report.frames_out,
            sample_rate_hz: report.sample_rate_hz,
            channels: report.channels,
            bits_out: report.bits_out,
            exact: report.exact,
            dithered: report.dithered,
            samples_saturated: report.samples_saturated,
            pcm_blake3: hex(&report.pcm_hash),
            blocks: BlockCounts {
                carried: report.count(BlockFate::Carried),
                patched: report.count(BlockFate::Patched),
                edited: report.count(BlockFate::Edited),
                replaced: report.count(BlockFate::Replaced),
                dropped: report.count(BlockFate::Dropped),
            },
            tags_added: report.tags_added,
            tags_not_added: report.tags_not_added.as_ref().map(ToString::to_string),
            stale_loudness_tags: report.stale_loudness_tags.clone(),
        }
    }
}

/// The request and what the render did: kept in the journal so recovery can write the sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The request.
    pub request: RenderRequest,
    /// What the render did.
    pub render: RenderSummary,
}

#[derive(Serialize)]
struct FileHash<'a> {
    blake3: &'a str,
    bytes: u64,
}

#[derive(Serialize)]
struct MetadataSummary<'a> {
    mtime_kept: bool,
    notes: &'a [String],
}

#[derive(Serialize)]
struct Sidecar<'a> {
    schema: u32,
    app: &'static str,
    version: &'static str,
    file: String,
    transaction: &'a str,
    processed_at: &'a str,
    mode: TxnKind,
    original: Option<FileHash<'a>>,
    output: Option<FileHash<'a>>,
    request: &'a RenderRequest,
    render: &'a RenderSummary,
    backup: Option<&'a Path>,
    metadata: MetadataSummary<'a>,
}

/// The sidecar path of `file`: `<file name>.soundcheck.json` next to it.
#[must_use]
pub fn sidecar_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(SIDECAR_SUFFIX);
    file.with_file_name(name)
}

/// The sidecar text of the finished transaction `entry` (keys in a fixed order, two-space
/// indent, a final newline); `notes` lists what the metadata step could not restore.
///
/// # Errors
/// [`Error::Internal`] when the entry has no record of its render.
pub(crate) fn render_text(entry: &Entry, notes: &[String]) -> Result<String> {
    let record = entry
        .record
        .as_ref()
        .ok_or_else(|| Error::Internal(format!("transaction {} has no render record", entry.txn)))?;
    let original = entry.original_blake3.as_deref().map(|h| FileHash {
        blake3: h,
        bytes: entry.original_bytes.unwrap_or(0),
    });
    let output = entry.output_blake3.as_deref().map(|h| FileHash {
        blake3: h,
        bytes: entry.output_bytes.unwrap_or(0),
    });
    let doc = Sidecar {
        schema: SIDECAR_SCHEMA,
        app: "SoundCheck",
        version: sc_core::VERSION,
        file: entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        transaction: &entry.txn,
        processed_at: &entry.started_at,
        mode: entry.kind,
        original,
        output,
        request: &record.request,
        render: &record.render,
        backup: entry.backup.as_deref(),
        metadata: MetadataSummary {
            mtime_kept: entry.keep_mtime,
            notes,
        },
    };
    let mut text = serde_json::to_string_pretty(&doc)
        .map_err(|e| Error::Internal(format!("sidecar not serialisable: {e}")))?;
    text.push('\n');
    Ok(text)
}

/// Writes the sidecar of `entry` next to its target, replacing an older one atomically.
///
/// # Errors
/// [`Error::Io`] naming the sidecar or its temp file.
pub(crate) fn write(entry: &Entry, notes: &[String]) -> Result<PathBuf> {
    let text = render_text(entry, notes)?;
    let path = sidecar_path(&entry.path);
    let dir = path.parent().unwrap_or(Path::new("."));
    let temp = dir.join(temp_name(
        path.file_name().unwrap_or_default(),
        &entry.txn,
    ));
    remove_if_exists(&temp)?;
    let written = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| io_err(&temp, e))?;
        f.write_all(text.as_bytes()).map_err(|e| io_err(&temp, e))?;
        sync_file(&f, &temp)?;
        drop(f);
        std::fs::rename(&temp, &path).map_err(|e| io_err(&path, e))
    })();
    if let Err(e) = written {
        let _ = remove_if_exists(&temp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(path)
}

/// Removes the sidecar of `file`; whether there was one.
///
/// # Errors
/// [`Error::Io`] naming the sidecar.
pub(crate) fn remove(file: &Path) -> Result<bool> {
    let path = sidecar_path(file);
    let removed = remove_if_exists(&path)?;
    if removed && let Some(dir) = path.parent() {
        sync_dir(dir);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
