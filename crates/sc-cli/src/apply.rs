//! `sc-cli apply` and `sc-cli undo`: changing files' level (and cutting their start) through the
//! write transaction, in place with a backup or as copies into a folder, and putting the
//! originals back. Files are processed one after the other in the order given; each prints one
//! line (or one JSON document) on success and three lines on stderr when it is refused.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sc_core::Error;
use sc_core::ipc::{IpcError, RecoveryStatus};
use sc_engine::txn::check_tags;
use sc_engine::{
    ApplyOptions, ApplyRequest, CancelToken, Place, Tag, apply_file, check_inputs, undo_file,
};
use sc_io::txn::sidecar::{BlockCounts, RenderSummary};
use sc_io::txn::{SidecarAfterUndo, TxnKind, TxnReport, UndoReport, hex};
use serde::Serialize;

use crate::refusal::{self, Action, Refusal};
use crate::vocab;

/// Version of the JSON documents of `apply`, `undo`, `journal` and `recover`.
pub const TXN_SCHEMA: u32 = 1;

/// Where backups and the journal live.
#[derive(clap::Args)]
pub struct BackupArgs {
    /// Folder of the backups and the journal (default: "SoundCheck Backups" in the Music
    /// folder, or `$SC_BACKUP_ROOT`).
    #[arg(long, value_name = "DIR")]
    pub backup_root: Option<PathBuf>,
}

impl BackupArgs {
    /// The backup root, absolute.
    ///
    /// # Errors
    /// When the system names no home folder, or the current folder cannot be read.
    pub fn resolve(&self) -> anyhow::Result<PathBuf> {
        let root = match &self.backup_root {
            Some(dir) => dir.clone(),
            None => sc_io::txn::default_backup_root()?,
        };
        Ok(std::path::absolute(root)?)
    }
}

/// `sc-cli apply`.
#[derive(clap::Args)]
pub struct ApplyArgs {
    /// WAV, AIFF or FLAC files.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Gain applied to every sample, dB (negative turns down; a boost that would clip is
    /// refused).
    #[arg(long, allow_hyphen_values = true, value_parser = parse_gain)]
    gain_db: f64,
    /// Samples per channel to cut from the start; the cut moves up to 1 ms earlier to the
    /// quietest frame (never later) and the next 2 ms fade in.
    #[arg(long, default_value_t = 0, value_name = "N")]
    trim_samples: u64,
    /// Output bits per sample (default: the source's; 24 for a float source).
    #[arg(long, value_parser = parse_bits)]
    bits: Option<u8>,
    /// Write copies into this folder (created if needed) and leave the files as they are.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    #[command(flatten)]
    backup: BackupArgs,
    /// Give the file the current time as its modification time (by default it keeps the
    /// original's, so DJ apps do not see a changed file).
    #[arg(long)]
    no_keep_mtime: bool,
    /// Write no `<file>.soundcheck.json` next to the file.
    #[arg(long)]
    no_sidecar: bool,
    /// A tag item to add or replace, by a container-neutral name: `BPM=128.00` (ID3 `TBPM` 128 and
    /// `TXXX:BPM` 128.00 in WAV/AIFF, `BPM` in FLAC), `INITIALKEY=8A` (`TKEY` / `INITIALKEY`),
    /// any other `NAME=VALUE` (`TXXX:NAME` / `NAME`). ID3 frame ids (`TBPM`) and labels with `:` are
    /// refused. Repeatable; tags go into an existing tag only, none is created.
    #[arg(long = "tag", value_name = "NAME=VALUE", value_parser = parse_tag)]
    tags: Vec<Tag>,
    /// Print JSON (one document per file) instead of text.
    #[arg(long)]
    json: bool,
}

/// `sc-cli undo`.
#[derive(clap::Args)]
pub struct UndoArgs {
    /// Files changed in place by `sc-cli apply` (or the app).
    #[arg(required = true)]
    files: Vec<PathBuf>,
    #[command(flatten)]
    backup: BackupArgs,
    /// Print JSON (one document per file) instead of text.
    #[arg(long)]
    json: bool,
}

fn parse_gain(text: &str) -> Result<f64, String> {
    let g: f64 = text
        .parse()
        .map_err(|e| format!("{text:?} is not a number: {e}"))?;
    if g.is_finite() && g.abs() <= 120.0 {
        Ok(g)
    } else {
        Err(format!("{text} dB is outside -120..120"))
    }
}

fn parse_bits(text: &str) -> Result<u8, String> {
    match text {
        "16" => Ok(16),
        "24" => Ok(24),
        _ => Err(format!("{text} is not 16 or 24")),
    }
}

fn parse_tag(text: &str) -> Result<Tag, String> {
    Tag::parse(text).map_err(|e| match e {
        Error::InvalidArgument(m) => m,
        other => other.to_string(),
    })
}

/// The JSON document of a file that was not changed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FailedDoc {
    schema: u32,
    file: String,
    ok: bool,
    error: IpcError,
    #[serde(flatten)]
    refusal: Refusal,
}

/// What `apply` was asked.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestDoc<'a> {
    gain_db: f64,
    trim_frames: u64,
    bits: Option<u8>,
    tags: &'a [Tag],
}

/// What the render did.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderDoc {
    frames_in: u64,
    frames_out: u64,
    trim_frames: u64,
    trim_requested_frames: u64,
    sample_rate_hz: u32,
    channels: u16,
    bits_out: u16,
    exact: bool,
    dithered: bool,
    samples_saturated: u64,
    pcm_blake3: String,
    blocks: BlockCounts,
    tags_added: u32,
    tags_replaced: u32,
    tags_not_added: Option<String>,
    stale_loudness_tags: Vec<String>,
}

/// One journaled step and the time it took.
#[derive(Serialize)]
struct StepDoc {
    step: &'static str,
    ms: f64,
}

/// The JSON document of a file `apply` changed or copied.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppliedDoc<'a> {
    schema: u32,
    file: String,
    ok: bool,
    txn: &'a str,
    kind: &'static str,
    output: String,
    backup: Option<String>,
    sidecar: Option<String>,
    request: RequestDoc<'a>,
    render: RenderDoc,
    original_blake3: String,
    output_blake3: String,
    output_bytes: u64,
    notes: &'a [String],
    timings: Vec<StepDoc>,
    total_ms: f64,
}

/// The JSON document of a file `undo` restored.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UndoneDoc<'a> {
    schema: u32,
    file: String,
    ok: bool,
    txn: &'a str,
    undone: &'a str,
    path: String,
    backup: String,
    restored_blake3: String,
    sidecar: &'static str,
    earlier_changes: usize,
    notes: &'a [String],
    total_ms: f64,
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

fn plural(n: impl Into<u64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Tag items appended and replaced.
fn tag_counts(r: &TxnReport) -> (u32, u32) {
    if let Some(e) = r.render.tag_edit {
        (e.appended, e.replaced)
    } else if let Some(e) = r.render.vorbis_edit {
        (e.appended, e.replaced)
    } else {
        (0, 0)
    }
}

/// The one line printed for a file `apply` changed or copied.
#[must_use]
pub fn applied_line(file: &Path, req: &ApplyRequest, r: &TxnReport) -> String {
    use std::fmt::Write as _;
    let render = &r.render;
    let summary = RenderSummary::of(render);
    let mut s = format!(
        "{}: gain {:+.2} dB, {}-bit",
        file.display(),
        req.gain_db,
        render.bits_out
    );
    if render.dithered {
        s.push_str(" dithered");
    }
    let _ = write!(
        s,
        ", {} trimmed",
        plural(render.trim_frames, "frame", "frames")
    );
    if render.trim_requested_frames != render.trim_frames {
        let _ = write!(s, " ({} requested)", render.trim_requested_frames);
    }
    if render.samples_saturated > 0 {
        let _ = write!(
            s,
            ", {} saturated",
            plural(render.samples_saturated, "sample", "samples")
        );
    }
    let blocks = summary.blocks;
    let _ = write!(
        s,
        "; {} carried",
        plural(blocks.carried as u64, "block", "blocks")
    );
    if blocks.patched > 0 {
        let _ = write!(s, ", {} patched", blocks.patched);
    }
    if blocks.dropped > 0 {
        let _ = write!(s, ", {} dropped", blocks.dropped);
    }
    let (added, replaced) = tag_counts(r);
    if added > 0 {
        let _ = write!(s, ", {} added", plural(added, "tag", "tags"));
    }
    if replaced > 0 {
        let _ = write!(s, ", {} replaced", plural(replaced, "tag", "tags"));
    }
    if let Some(why) = &render.tags_not_added {
        let _ = write!(s, "; tags not added: {why}");
    }
    if !render.stale_loudness_tags.is_empty() {
        let _ = write!(
            s,
            "; stale loudness tags: {}",
            render.stale_loudness_tags.join(", ")
        );
    }
    s.push_str("; verified");
    match (&r.backup, r.kind) {
        (Some(b), _) => {
            let _ = write!(s, "; backup {}", b.display());
        }
        (None, TxnKind::ToFolder) => {
            let _ = write!(s, "; written to {}", r.output.display());
        }
        (None, _) => {}
    }
    for note in &r.notes {
        let _ = write!(s, "; note: {note}");
    }
    s
}

fn applied_doc<'a>(
    file: &Path,
    req: &'a ApplyRequest,
    r: &'a TxnReport,
    total: Duration,
) -> AppliedDoc<'a> {
    let summary = RenderSummary::of(&r.render);
    let (tags_added, tags_replaced) = tag_counts(r);
    AppliedDoc {
        schema: TXN_SCHEMA,
        file: file.display().to_string(),
        ok: true,
        txn: &r.txn,
        kind: vocab::kind(r.kind),
        output: r.output.display().to_string(),
        backup: r.backup.as_ref().map(|p| p.display().to_string()),
        sidecar: r.sidecar.as_ref().map(|p| p.display().to_string()),
        request: RequestDoc {
            gain_db: req.gain_db,
            trim_frames: req.trim_frames,
            bits: req.bits,
            tags: &req.tags,
        },
        render: RenderDoc {
            frames_in: summary.frames_in,
            frames_out: summary.frames_out,
            trim_frames: summary.trim_frames,
            trim_requested_frames: summary.trim_requested_frames,
            sample_rate_hz: summary.sample_rate_hz,
            channels: summary.channels,
            bits_out: summary.bits_out,
            exact: summary.exact,
            dithered: summary.dithered,
            samples_saturated: summary.samples_saturated,
            pcm_blake3: summary.pcm_blake3,
            blocks: summary.blocks,
            tags_added,
            tags_replaced,
            tags_not_added: summary.tags_not_added,
            stale_loudness_tags: summary.stale_loudness_tags,
        },
        original_blake3: hex(&r.original_blake3),
        output_blake3: hex(&r.output_blake3),
        output_bytes: r.output_bytes,
        notes: &r.notes,
        timings: r
            .timings
            .iter()
            .map(|(state, d)| StepDoc {
                step: vocab::state(*state),
                ms: ms(*d),
            })
            .collect(),
        total_ms: ms(total),
    }
}

/// Prints a refusal: three lines on stderr, and the JSON document on stdout with `--json`.
fn print_failed(file: &Path, err: Error, action: Action, json: bool) -> anyhow::Result<()> {
    let why = refusal::explain(&err, action);
    refusal::write(&mut std::io::stderr().lock(), file, action, &why)?;
    if json {
        let doc = FailedDoc {
            schema: TXN_SCHEMA,
            file: file.display().to_string(),
            ok: false,
            error: IpcError::from(err),
            refusal: why,
        };
        let mut out = std::io::stdout().lock();
        serde_json::to_writer_pretty(&mut out, &doc)?;
        writeln!(out)?;
    }
    Ok(())
}

/// Runs recovery before a command that writes, saying on stderr what it found.
pub fn recover_first(root: &Path) {
    match sc_engine::recover_at_start(root) {
        RecoveryStatus::Finished { recovered, pending } => {
            if !recovered.is_empty() {
                eprintln!(
                    "sc-cli: finished or rolled back {} interrupted (see sc-cli journal)",
                    plural(recovered.len() as u64, "change", "changes")
                );
            }
            if !pending.is_empty() {
                eprintln!(
                    "sc-cli: {} still pending (see sc-cli recover)",
                    plural(pending.len() as u64, "change is", "changes are")
                );
            }
        }
        RecoveryStatus::Failed { message } => {
            eprintln!("sc-cli: recovery could not run: {message}");
        }
        RecoveryStatus::Running | RecoveryStatus::Skipped { .. } => {}
    }
}

/// Stops the run before any file when the tags are not valid (exit code 2).
fn check_tags_or_exit(tags: &[Tag]) {
    if let Err(err) = check_tags(tags) {
        eprintln!("sc-cli: {err}");
        std::process::exit(crate::EXIT_FAILED);
    }
}

/// `sc-cli apply`: returns how many files failed.
///
/// # Errors
/// When the backup root cannot be named or the output cannot be written.
pub fn run_apply(args: ApplyArgs) -> anyhow::Result<usize> {
    check_tags_or_exit(&args.tags);
    let root = args.backup.resolve()?;
    recover_first(&root);
    let req = ApplyRequest {
        gain_db: args.gain_db,
        trim_frames: args.trim_samples,
        bits: args.bits,
        loudness: None,
        tags: args.tags,
    };
    let (place, action) = match args.out {
        Some(dir) => (Place::Folder(std::path::absolute(dir)?), Action::Copy),
        None => (Place::InPlace, Action::Change),
    };
    // Files listed twice and copies that would share a name are refused before any write.
    let refused = check_inputs(&args.files, &place);
    let opts = ApplyOptions {
        place,
        backup_root: root,
        keep_mtime: !args.no_keep_mtime,
        sidecar: !args.no_sidecar,
    };
    let cancel = CancelToken::new();
    let mut failed = 0;
    for (file, refusal) in args.files.iter().zip(refused) {
        let started = Instant::now();
        let result = match refusal {
            Some(err) => Err(err),
            None => apply_file(file, &req, &opts, &cancel),
        };
        match result {
            Ok(report) => {
                let mut out = std::io::stdout().lock();
                if args.json {
                    let doc = applied_doc(file, &req, &report, started.elapsed());
                    serde_json::to_writer_pretty(&mut out, &doc)?;
                    writeln!(out)?;
                } else {
                    writeln!(out, "{}", applied_line(file, &req, &report))?;
                }
            }
            Err(err) => {
                failed += 1;
                print_failed(file, err, action, args.json)?;
            }
        }
    }
    Ok(failed)
}

/// The one line printed for a file `undo` restored.
#[must_use]
pub fn undone_line(file: &Path, r: &UndoReport) -> String {
    use std::fmt::Write as _;
    let sidecar = match &r.sidecar {
        SidecarAfterUndo::Removed => "; sidecar removed",
        SidecarAfterUndo::Restored(_) => "; sidecar describes the previous change again",
        SidecarAfterUndo::Absent => "",
    };
    let earlier = match r.earlier_changes {
        0 => String::new(),
        n => format!(
            "; {} (undo again to go further back)",
            plural(n as u64, "earlier change remains", "earlier changes remain")
        ),
    };
    let mut s = format!(
        "{}: previous version restored from {}; verified{sidecar}{earlier}",
        file.display(),
        r.backup.display()
    );
    for note in &r.notes {
        let _ = write!(s, "; note: {note}");
    }
    s
}

fn undone_doc<'a>(file: &Path, r: &'a UndoReport, total: Duration) -> UndoneDoc<'a> {
    UndoneDoc {
        schema: TXN_SCHEMA,
        file: file.display().to_string(),
        ok: true,
        txn: &r.txn,
        undone: &r.undone,
        path: r.path.display().to_string(),
        backup: r.backup.display().to_string(),
        restored_blake3: hex(&r.restored_blake3),
        sidecar: match r.sidecar {
            SidecarAfterUndo::Removed => "removed",
            SidecarAfterUndo::Restored(_) => "restored",
            SidecarAfterUndo::Absent => "absent",
        },
        earlier_changes: r.earlier_changes,
        notes: &r.notes,
        total_ms: ms(total),
    }
}

/// `sc-cli undo`: returns how many files failed.
///
/// # Errors
/// When the backup root cannot be named or the output cannot be written.
pub fn run_undo(args: &UndoArgs) -> anyhow::Result<usize> {
    let root = args.backup.resolve()?;
    recover_first(&root);
    let refused = check_inputs(&args.files, &Place::InPlace);
    let mut failed = 0;
    for (file, refusal) in args.files.iter().zip(refused) {
        let started = Instant::now();
        let result = match refusal {
            Some(err) => Err(err),
            None => undo_file(file, &root),
        };
        match result {
            Ok(report) => {
                let mut out = std::io::stdout().lock();
                if args.json {
                    let doc = undone_doc(file, &report, started.elapsed());
                    serde_json::to_writer_pretty(&mut out, &doc)?;
                    writeln!(out)?;
                } else {
                    writeln!(out, "{}", undone_line(file, &report))?;
                }
            }
            Err(err) => {
                failed += 1;
                print_failed(file, err, Action::Undo, args.json)?;
            }
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests;
