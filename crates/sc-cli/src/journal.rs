//! `sc-cli journal` and `sc-cli recover`: the changes recorded in the backup root's journal,
//! newest first; giving up on a pending one; and running crash recovery by hand.

use std::io::Write;
use std::path::Path;

use sc_core::ipc::IpcError;
use sc_io::txn::{self, Entry, Outcome, RecoveryReport, State, TxnKind};
use serde::Serialize;

use crate::apply::{BackupArgs, TXN_SCHEMA};
use crate::vocab;

/// `sc-cli journal`.
#[derive(clap::Args)]
pub struct JournalArgs {
    /// Only changes that have not ended: recovery finishes or rolls them back.
    #[arg(long, conflicts_with = "forget")]
    incomplete: bool,
    /// Give up on the pending change with this id: recovery skips it from then on, and the
    /// backup and every file it left are kept where they are.
    #[arg(long, value_name = "ID")]
    forget: Option<String>,
    #[command(flatten)]
    backup: BackupArgs,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
}

/// `sc-cli recover`.
#[derive(clap::Args)]
pub struct RecoverArgs {
    #[command(flatten)]
    backup: BackupArgs,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
}

/// One change as `journal --json` prints it (kinds, states and outcomes in [`vocab`]'s words).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryDoc<'a> {
    txn: &'a str,
    started_at: &'a str,
    kind: &'static str,
    state: &'static str,
    reached: &'static str,
    outcome: Option<&'static str>,
    undone: bool,
    path: String,
    source: String,
    backup: Option<String>,
    error: Option<&'a str>,
    notes: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JournalDoc<'a> {
    schema: u32,
    backup_root: String,
    entries: Vec<EntryDoc<'a>>,
}

fn kind_text(kind: TxnKind) -> &'static str {
    match kind {
        TxnKind::InPlace => "in place",
        TxnKind::ToFolder => "copy",
        TxnKind::Undo => "undo",
    }
}

/// The state column: how the change ended, or where it stopped.
fn status(e: &Entry) -> String {
    let undone = if e.undone { ", undone" } else { "" };
    match (e.state, e.outcome) {
        (State::Done, _) => format!("done{undone}"),
        (State::Failed, _) => "failed".to_owned(),
        (State::Recovered, Some(Outcome::Completed)) => format!("recovered: completed{undone}"),
        (State::Recovered, _) => "recovered: rolled back".to_owned(),
        (State::Forgotten, _) => format!("forgotten at {}", e.reached.as_str()),
        (_, _) => format!("pending at {}", e.reached.as_str()),
    }
}

/// The backup an entry made or restored, when it has one.
fn backup_of(e: &Entry) -> Option<&Path> {
    e.backup.as_deref().or(e.backup_target.as_deref())
}

/// The text lines of one entry.
fn write_entry(out: &mut impl Write, e: &Entry) -> std::io::Result<()> {
    writeln!(
        out,
        "{}  {}  {}  {}  {}",
        e.started_at,
        e.txn,
        kind_text(e.kind),
        status(e),
        e.path.display()
    )?;
    if let Some(b) = backup_of(e) {
        let label = if e.kind == TxnKind::Undo {
            "from backup"
        } else {
            "backup"
        };
        writeln!(out, "    {label} {}", b.display())?;
    }
    if let Some(err) = &e.error {
        writeln!(out, "    error: {err}")?;
    }
    Ok(())
}

/// `sc-cli journal`: returns how many requests failed (a refused `--forget`).
///
/// # Errors
/// When the backup root cannot be named, the journal cannot be read or the output written.
pub fn run_journal(args: &JournalArgs) -> anyhow::Result<usize> {
    let root = args.backup.resolve()?;
    if let Some(id) = &args.forget {
        return forget(&root, id, args.json);
    }
    let mut entries = txn::journal_entries(&root)?;
    entries.reverse();
    if args.incomplete {
        entries.retain(|e| !e.state.is_final());
    }
    let mut out = std::io::stdout().lock();
    if args.json {
        let doc = JournalDoc {
            schema: TXN_SCHEMA,
            backup_root: root.display().to_string(),
            entries: entries
                .iter()
                .map(|e| EntryDoc {
                    txn: &e.txn,
                    started_at: &e.started_at,
                    kind: vocab::kind(e.kind),
                    state: vocab::state(e.state),
                    reached: vocab::state(e.reached),
                    outcome: e.outcome.map(vocab::outcome),
                    undone: e.undone,
                    path: e.path.display().to_string(),
                    source: e.source.display().to_string(),
                    backup: backup_of(e).map(|b| b.display().to_string()),
                    error: e.error.as_deref(),
                    notes: &e.notes,
                })
                .collect(),
        };
        serde_json::to_writer_pretty(&mut out, &doc)?;
        writeln!(out)?;
    } else if entries.is_empty() {
        let what = if args.incomplete {
            "no unfinished changes"
        } else {
            "no changes"
        };
        writeln!(out, "{what} recorded in {}", root.display())?;
    } else {
        for e in &entries {
            write_entry(&mut out, e)?;
        }
    }
    Ok(0)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForgottenDoc {
    schema: u32,
    ok: bool,
    txn: String,
    kind: &'static str,
    path: String,
    reached: &'static str,
    backup: Option<String>,
    kept: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForgetFailedDoc {
    schema: u32,
    ok: bool,
    txn: String,
    error: IpcError,
}

fn forget(root: &Path, id: &str, json: bool) -> anyhow::Result<usize> {
    let mut out = std::io::stdout().lock();
    let f = match txn::forget(root, id) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("sc-cli: {err}");
            if json {
                let doc = ForgetFailedDoc {
                    schema: TXN_SCHEMA,
                    ok: false,
                    txn: id.to_owned(),
                    error: IpcError::from(err),
                };
                serde_json::to_writer_pretty(&mut out, &doc)?;
                writeln!(out)?;
            }
            return Ok(1);
        }
    };
    if json {
        let doc = ForgottenDoc {
            schema: TXN_SCHEMA,
            ok: true,
            txn: f.txn,
            kind: vocab::kind(f.kind),
            path: f.path.display().to_string(),
            reached: vocab::state(f.reached),
            backup: f.backup.map(|b| b.display().to_string()),
            kept: f.kept.iter().map(|p| p.display().to_string()).collect(),
        };
        serde_json::to_writer_pretty(&mut out, &doc)?;
        writeln!(out)?;
        return Ok(0);
    }
    writeln!(
        out,
        "forgot {} ({}, {}, stopped at {}); recovery skips it from now on",
        f.txn,
        f.path.display(),
        kind_text(f.kind),
        f.reached.as_str()
    )?;
    writeln!(out, "  nothing was deleted")?;
    if let Some(b) = &f.backup {
        writeln!(out, "  the original's backup is kept at {}", b.display())?;
    }
    for p in f.kept.iter().filter(|p| Some(*p) != f.backup.as_ref()) {
        writeln!(out, "  also left at {}", p.display())?;
    }
    Ok(0)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveredDoc<'a> {
    txn: &'a str,
    kind: &'static str,
    path: String,
    reached: &'static str,
    outcome: &'static str,
    notes: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingDoc<'a> {
    txn: &'a str,
    path: String,
    reason: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryDoc<'a> {
    schema: u32,
    backup_root: String,
    recovered: Vec<RecoveredDoc<'a>>,
    pending: Vec<PendingDoc<'a>>,
}

fn write_recovery(
    out: &mut impl Write,
    root: &Path,
    report: &RecoveryReport,
) -> std::io::Result<()> {
    if report.recovered.is_empty() && report.pending.is_empty() {
        return writeln!(out, "nothing to recover in {}", root.display());
    }
    if !report.recovered.is_empty() {
        writeln!(
            out,
            "recovered {} interrupted:",
            match report.recovered.len() {
                1 => "1 change".to_owned(),
                n => format!("{n} changes"),
            }
        )?;
    }
    for r in &report.recovered {
        let outcome = match r.outcome {
            Outcome::Completed => "completed",
            Outcome::RolledBack => "rolled back, the file is as it was",
        };
        writeln!(
            out,
            "  {}  {}: {outcome} (it had reached {})",
            r.txn,
            r.path.display(),
            r.reached.as_str()
        )?;
        for note in &r.notes {
            writeln!(out, "    note: {note}")?;
        }
    }
    if !report.pending.is_empty() {
        writeln!(out, "left pending (tried again next time):")?;
    }
    for p in &report.pending {
        writeln!(out, "  {}  {}: {}", p.txn, p.path.display(), p.reason)?;
        writeln!(
            out,
            "    to give up on it: sc-cli journal --forget {}",
            p.txn
        )?;
    }
    Ok(())
}

/// Exit code of `recover` when changes stay pending.
pub const EXIT_PENDING: i32 = 3;

/// `sc-cli recover`: returns whether every interrupted change was ended (none left pending).
///
/// # Errors
/// When the backup root cannot be named, the journal cannot be read or the output written.
pub fn run_recover(args: &RecoverArgs) -> anyhow::Result<bool> {
    let root = args.backup.resolve()?;
    let report = txn::recover(&root)?;
    let cache = sc_io::cache::Cache::open(sc_io::cache::Cache::default_dir()?);
    sc_engine::forget_recovered(Some(&cache), &report);
    let mut out = std::io::stdout().lock();
    if args.json {
        let doc = RecoveryDoc {
            schema: TXN_SCHEMA,
            backup_root: root.display().to_string(),
            recovered: report
                .recovered
                .iter()
                .map(|r| RecoveredDoc {
                    txn: &r.txn,
                    kind: vocab::kind(r.kind),
                    path: r.path.display().to_string(),
                    reached: vocab::state(r.reached),
                    outcome: vocab::outcome(r.outcome),
                    notes: &r.notes,
                })
                .collect(),
            pending: report
                .pending
                .iter()
                .map(|p| PendingDoc {
                    txn: &p.txn,
                    path: p.path.display().to_string(),
                    reason: &p.reason,
                })
                .collect(),
        };
        serde_json::to_writer_pretty(&mut out, &doc)?;
        writeln!(out)?;
    } else {
        write_recovery(&mut out, &root, &report)?;
    }
    Ok(report.pending.is_empty())
}
