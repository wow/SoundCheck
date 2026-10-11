//! The sidecar `<file>.soundcheck.json` written next to every processed file: what was asked,
//! what the render did, the hashes of the original and the output, where the backup is and,
//! for a file an export wrote, what the export planned and the grid it exported ([`SidecarDoc`]).
//!
//! Keys come in a fixed order and every value derives from the input, the request and the
//! version, except `transaction` and `processed_at`, which say which run wrote it. The file is
//! replaced atomically (temp file, sync, rename), so a reader never sees half of one. [`read`]
//! reads every schema written so far.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use sc_core::export::{ExportRecord, GridCheck};
use sc_core::{Error, RenderRequest, Result};

use super::fsx::{hex, io_err, remove_if_exists, sync_dir, sync_file, temp_name};
use super::journal::{Entry, Journal, Line, State, TxnKind};
use crate::render::{BlockFate, RenderReport};

/// Version of the sidecar layout; a change that breaks readers increments it. Version 2 added
/// `export`; version 1 sidecars (without it) still read.
pub const SIDECAR_SCHEMA: u32 = 2;

/// The oldest sidecar layout [`read`] reads.
pub const OLDEST_SIDECAR_SCHEMA: u32 = 1;

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
    /// Frames cut from the start (the requested cut snapped back by up to 1 ms). Always
    /// written; `None` only in records written before it was recorded, whose cut
    /// [`Self::trim_frames`] derives from the frame counts.
    #[serde(default)]
    pub trim_frames: Option<u64>,
    /// Frames the request asked to cut. Always written; `None` only in records written before
    /// cuts were snapped, which cut exactly what was asked ([`Self::trim_requested_frames`]).
    #[serde(default)]
    pub trim_requested_frames: Option<u64>,
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
            trim_frames: Some(report.trim_frames),
            trim_requested_frames: Some(report.trim_requested_frames),
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

    /// Frames cut from the start: as recorded, or, in a record from before the cut was
    /// recorded, `frames_in - frames_out` (a render changes the length by its cut alone).
    #[must_use]
    pub fn trim_frames(&self) -> u64 {
        self.trim_frames
            .unwrap_or_else(|| self.frames_in.saturating_sub(self.frames_out))
    }

    /// Frames the request asked to cut: as recorded, or, in a record from before cuts were
    /// snapped, the cut made ([`Self::trim_frames`]).
    #[must_use]
    pub fn trim_requested_frames(&self) -> u64 {
        self.trim_requested_frames
            .unwrap_or_else(|| self.trim_frames())
    }
}

/// The request and what the render did: kept in the journal so recovery can write the sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The request.
    pub request: RenderRequest,
    /// What the render did.
    pub render: RenderSummary,
    /// What the export that asked for the render planned; `None` for a plain render (and in
    /// records written before exports were recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportRecord>,
}

/// A file's length and hash, as a sidecar records them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHash {
    /// BLAKE3, hex.
    pub blake3: String,
    /// Length, bytes.
    pub bytes: u64,
}

/// What became of the original's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSummary {
    /// The modification time was put back.
    pub mtime_kept: bool,
    /// What the backup lacks and what could not be restored.
    pub notes: Vec<String>,
}

/// A sidecar as written and as [`read`] reads it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarDoc {
    /// The layout version, [`OLDEST_SIDECAR_SCHEMA`] to [`SIDECAR_SCHEMA`].
    pub schema: u32,
    /// `SoundCheck`.
    pub app: String,
    /// The SoundCheck version that wrote it.
    pub version: String,
    /// The file's name.
    pub file: String,
    /// The transaction that wrote the file.
    pub transaction: String,
    /// When the transaction started (RFC 3339, UTC).
    pub processed_at: String,
    /// In place or into a folder.
    pub mode: TxnKind,
    /// The original file.
    pub original: Option<FileHash>,
    /// The file written.
    pub output: Option<FileHash>,
    /// What the render was asked.
    pub request: RenderRequest,
    /// What the render did.
    pub render: RenderSummary,
    /// The backup of the original (in place).
    pub backup: Option<PathBuf>,
    /// What became of the original's metadata.
    pub metadata: MetadataSummary,
    /// What the export planned and the grid it exported; absent for a plain render and in
    /// schema 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportRecord>,
}

/// The sidecar path of `file`: `<file name>.soundcheck.json` next to it.
#[must_use]
pub fn sidecar_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(SIDECAR_SUFFIX);
    file.with_file_name(name)
}

/// The sidecar text of the finished transaction `entry` (keys in a fixed order, two-space
/// indent, a final newline); `notes` lists what the backup lacks and what the metadata step
/// could not restore.
///
/// # Errors
/// [`Error::Internal`] when the entry has no record of its render.
pub(crate) fn render_text(entry: &Entry, notes: &[String]) -> Result<String> {
    let record = entry.record.as_ref().ok_or_else(|| {
        Error::Internal(format!("transaction {} has no render record", entry.txn))
    })?;
    let hash = |h: Option<&String>, bytes: Option<u64>| {
        h.map(|h| FileHash {
            blake3: h.clone(),
            bytes: bytes.unwrap_or(0),
        })
    };
    let doc = SidecarDoc {
        schema: SIDECAR_SCHEMA,
        app: "SoundCheck".to_owned(),
        version: sc_core::VERSION.to_owned(),
        file: entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        transaction: entry.txn.clone(),
        processed_at: entry.started_at.clone(),
        mode: entry.kind,
        original: hash(entry.original_blake3.as_ref(), entry.original_bytes),
        output: hash(entry.output_blake3.as_ref(), entry.output_bytes),
        request: record.request.clone(),
        render: record.render.clone(),
        backup: entry.backup.clone(),
        metadata: MetadataSummary {
            mtime_kept: entry.keep_mtime,
            notes: notes.to_vec(),
        },
        export: record.export.clone(),
    };
    doc_text(&doc)
}

/// Reads the sidecar at `path` (see [`sidecar_path`]): any schema from
/// [`OLDEST_SIDECAR_SCHEMA`] to [`SIDECAR_SCHEMA`]. Values a schema did not have yet read as
/// absent (a render's cut, [`RenderSummary::trim_frames`]; the `export`).
///
/// # Errors
/// [`Error::Io`] when it cannot be read; [`Error::Corrupt`] when it is not a sidecar, or one of
/// a schema this version does not read.
pub fn read(path: &Path) -> Result<SidecarDoc> {
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let corrupt = |detail: String| Error::Corrupt {
        path: path.to_path_buf(),
        detail,
    };
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| corrupt(format!("not JSON: {e}")))?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| corrupt("no schema".to_owned()))?;
    if !(u64::from(OLDEST_SIDECAR_SCHEMA)..=u64::from(SIDECAR_SCHEMA)).contains(&schema) {
        return Err(corrupt(format!(
            "sidecar schema {schema}; this version reads {OLDEST_SIDECAR_SCHEMA} to {SIDECAR_SCHEMA}"
        )));
    }
    serde_json::from_value(value).map_err(|e| corrupt(format!("not a sidecar: {e}")))
}

/// The text of `doc` as sidecars are written: keys in a fixed order, two-space indent, a final
/// newline.
fn doc_text(doc: &SidecarDoc) -> Result<String> {
    let mut text = serde_json::to_string_pretty(doc)
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
    replace(&path, &text, &entry.txn)?;
    Ok(path)
}

/// Records `check`, the check of the grid exported by the transaction `txn` that wrote `file`:
/// journaled in `backup_root` first (so a sidecar written again from the journal, as an undo of
/// a later change does, keeps it), then added to the file's sidecar, which is otherwise left
/// byte for byte as it was. A sidecar of another transaction (a later change wrote the file)
/// or without an export is left alone.
///
/// # Errors
/// [`Error::Io`] when the journal or the sidecar cannot be read or written; [`Error::Corrupt`]
/// when the sidecar does not read.
pub fn record_grid_check(
    backup_root: &Path,
    txn: &str,
    file: &Path,
    check: &GridCheck,
) -> Result<()> {
    let mut line = Line::new(txn, State::Done);
    line.grid_check = Some(check.clone());
    Journal::at(backup_root).append(&line)?;
    let path = sidecar_path(file);
    let mut doc = read(&path)?;
    if doc.transaction != txn {
        return Ok(());
    }
    let Some(export) = doc.export.as_mut() else {
        return Ok(());
    };
    export.grid_check = Some(check.clone());
    replace(&path, &doc_text(&doc)?, txn)
}

/// Replaces the file at `path` with `text` atomically (a temp file named for `txn`, synced,
/// renamed over it, the folder synced).
fn replace(path: &Path, text: &str, txn: &str) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let temp = dir.join(temp_name(path.file_name().unwrap_or_default(), txn));
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
        std::fs::rename(&temp, path).map_err(|e| io_err(path, e))
    })();
    if let Err(e) = written {
        let _ = remove_if_exists(&temp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(())
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
