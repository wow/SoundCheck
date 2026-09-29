//! `sc-cli eval`: scores the analysis against hand labels.
//!
//! The labels file is CSV with a header (lines starting with `#` are comments); recognised columns (any order, all but `file`
//! optional): `file` (base name, matched NFC-normalised anywhere under the folder), `bpm`,
//! `bar1_s` (bar 1 in seconds from the start of the decoded audio), `meter` (`4/4`, `9/8`, ...),
//! `grouping` (`2+2+2+3`), `ffmpeg_i_lufs` (integrated loudness measured by
//! `scripts/ffmpeg-reference.sh`), `fit` (`whole`, the default, or `start`: the label was set on
//! a grid fitted to the start of a track whose tempo changes, so it is scored against the fresh
//! analysis refitted the same way). Each row is scored where both sides exist:
//!
//! - BPM within 0.02, exactly or after one octave flip (x2 or /2);
//! - meter and grouping identical;
//! - bar 1 within 15 ms, compared modulo the bar length (bar 1 of a lead-in may be a whole bar
//!   apart and still be the same downbeat): the label's meter (ours when it has none) at the
//!   label's tempo; without a labelled tempo bar 1 is not scored;
//! - integrated loudness within 0.1 LU of ffmpeg.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use sc_core::Bpm;
use sc_core::analysis::{GridEdit, GridFit};
use sc_engine::Analyzer;
use sc_engine::edits::refit_record;

/// Tolerances of the scores.
const BPM_TOLERANCE: f64 = 0.02;
const BAR1_TOLERANCE_MS: f64 = 15.0;
const LOUDNESS_TOLERANCE_LU: f64 = 0.1;

/// One labelled track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Label {
    /// Base name of the file.
    pub file: String,
    /// Tempo at the meter's beat.
    pub bpm: Option<f64>,
    /// Bar 1 in seconds.
    pub bar1_s: Option<f64>,
    /// Meter as `n/d`.
    pub meter: Option<String>,
    /// Pulses per group, e.g. `[2, 2, 2, 3]`.
    pub grouping: Option<Vec<u8>>,
    /// Reference integrated loudness.
    pub ffmpeg_i_lufs: Option<f64>,
    /// The part of the track the labelled grid was fitted to (whole when not given).
    pub fit: GridFit,
}

/// One scored row.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scored {
    /// Base name.
    pub file: String,
    /// Why the row could not be scored.
    pub skipped: Option<String>,
    /// Label and result.
    pub bpm_label: Option<f64>,
    /// Our tempo.
    pub bpm: Option<f64>,
    /// The fit our grid was refitted with (the label's).
    pub fit: Option<GridFit>,
    /// Within tolerance without a flip.
    pub bpm_exact: Option<bool>,
    /// Within tolerance after one octave flip.
    pub bpm_after_flip: Option<bool>,
    /// Labelled meter with grouping.
    pub meter_label: Option<String>,
    /// Our meter with grouping.
    pub meter: Option<String>,
    /// Identical.
    pub meter_ok: Option<bool>,
    /// Bar-1 difference modulo the bar, in milliseconds.
    pub bar1_diff_ms: Option<f64>,
    /// Within tolerance.
    pub bar1_ok: Option<bool>,
    /// Ours minus ffmpeg, in LU.
    pub loudness_diff_lu: Option<f64>,
    /// Within tolerance.
    pub loudness_ok: Option<bool>,
}

/// Counts per score.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Rows in the labels file.
    pub rows: usize,
    /// Rows whose file was found and analysed.
    pub analysed: usize,
    /// BPM within tolerance / labelled.
    pub bpm_exact: (usize, usize),
    /// BPM within tolerance after at most one flip / labelled.
    pub bpm_after_flip: (usize, usize),
    /// Meter identical / labelled.
    pub meter: (usize, usize),
    /// Bar 1 within tolerance / labelled.
    pub bar1: (usize, usize),
    /// Integrated loudness within tolerance / labelled.
    pub loudness: (usize, usize),
}

/// Parses the labels CSV.
///
/// # Errors
/// A message when the header lacks `file` or a number does not parse.
pub fn parse_labels(text: &str) -> Result<Vec<Label>, String> {
    let mut lines = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let header = split_csv(lines.next().ok_or("empty labels file")?);
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let file_col = col("file").ok_or("labels need a `file` column")?;
    let (bpm_col, bar1_col, meter_col, grouping_col, loud_col, fit_col) = (
        col("bpm"),
        col("bar1_s"),
        col("meter"),
        col("grouping"),
        col("ffmpeg_i_lufs"),
        col("fit"),
    );
    let mut labels = Vec::new();
    for (n, line) in lines.enumerate() {
        let cells = split_csv(line);
        let get = |c: Option<usize>| {
            c.and_then(|i| cells.get(i))
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        let number = |c: Option<usize>, what: &str| -> Result<Option<f64>, String> {
            get(c)
                .map(|s| {
                    s.parse::<f64>()
                        .map_err(|e| format!("row {}: {what} {s:?}: {e}", n + 2))
                })
                .transpose()
        };
        let grouping = get(grouping_col)
            .map(|g| {
                g.split('+')
                    .map(|p| p.trim().parse::<u8>())
                    .collect::<Result<Vec<u8>, _>>()
                    .map_err(|e| format!("row {}: grouping {g:?}: {e}", n + 2))
            })
            .transpose()?;
        let fit = match get(fit_col).map(|f| f.to_ascii_lowercase()).as_deref() {
            None | Some("whole") => GridFit::Whole,
            Some("start") => GridFit::Start,
            Some(other) => {
                return Err(format!("row {}: fit {other:?} (whole or start)", n + 2));
            }
        };
        labels.push(Label {
            file: get(Some(file_col)).unwrap_or_default().nfc().collect(),
            bpm: number(bpm_col, "bpm")?,
            bar1_s: number(bar1_col, "bar1_s")?,
            meter: get(meter_col),
            grouping,
            ffmpeg_i_lufs: number(loud_col, "ffmpeg_i_lufs")?,
            fit,
        });
    }
    Ok(labels)
}

/// Splits one CSV line, honouring double quotes.
fn split_csv(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => cells.push(std::mem::take(&mut cell)),
            _ => cell.push(c),
        }
    }
    cells.push(cell);
    cells
}

/// Every file under `dir`, by NFC base name.
fn index_dir(dir: &Path) -> BTreeMap<String, PathBuf> {
    let mut found = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                found.insert(name.nfc().collect(), path);
            }
        }
    }
    found
}

/// Analyses and scores every labelled file found under `dir`; a `start` label is scored against
/// the analysis refitted to the start under `bpm_range` (the analysis's range).
pub fn run(
    analyzer: &mut Analyzer,
    labels: &[Label],
    dir: &Path,
    bpm_range: (Bpm, Bpm),
) -> (Vec<Scored>, Summary) {
    let files = index_dir(dir);
    let mut summary = Summary {
        rows: labels.len(),
        ..Summary::default()
    };
    let mut rows = Vec::with_capacity(labels.len());
    for label in labels {
        let mut row = Scored {
            file: label.file.clone(),
            bpm_label: label.bpm,
            meter_label: meter_text(label.meter.as_deref(), label.grouping.as_deref()),
            ..Scored::default()
        };
        let Some(path) = files.get(&label.file) else {
            row.skipped = Some("not found".into());
            rows.push(row);
            continue;
        };
        let report = match analyzer.analyze(path) {
            Ok(r) => r,
            Err(e) => {
                row.skipped = Some(e.to_string());
                rows.push(row);
                continue;
            }
        };
        summary.analysed += 1;
        let record = &report.record;
        if let (Some(reference), Some(i)) = (label.ffmpeg_i_lufs, record.loudness.integrated) {
            let diff = i.0 - reference;
            row.loudness_diff_lu = Some(diff);
            row.loudness_ok = Some(diff.abs() <= LOUDNESS_TOLERANCE_LU);
            tally(&mut summary.loudness, row.loudness_ok);
        }
        let start_fit;
        let grid = match (&record.grid, label.fit) {
            (None, _) => {
                row.skipped.clone_from(&record.grid_skipped);
                rows.push(row);
                continue;
            }
            (Some(grid), GridFit::Whole) => grid,
            (Some(_), GridFit::Start) => {
                let edit = GridEdit {
                    fit: GridFit::Start,
                    ..GridEdit::default()
                };
                let Some(solved) = refit_record(record, bpm_range, &edit) else {
                    row.skipped = Some("the start fit gives no grid".into());
                    rows.push(row);
                    continue;
                };
                start_fit = solved.grid;
                &start_fit
            }
        };
        row.fit = Some(label.fit);
        let bpm = grid.bpm.0;
        row.bpm = Some(bpm);
        row.meter = Some(grid.meter.to_string());
        if let Some(label_bpm) = label.bpm {
            let exact = (bpm - label_bpm).abs() <= BPM_TOLERANCE;
            let flipped = [2.0, 0.5]
                .iter()
                .any(|k| (bpm * k - label_bpm).abs() <= BPM_TOLERANCE);
            row.bpm_exact = Some(exact);
            row.bpm_after_flip = Some(exact || flipped);
            tally(&mut summary.bpm_exact, row.bpm_exact);
            tally(&mut summary.bpm_after_flip, row.bpm_after_flip);
        }
        if let Some(expected) = &row.meter_label {
            row.meter_ok = Some(row.meter.as_deref() == Some(expected.as_str()));
            tally(&mut summary.meter, row.meter_ok);
        }
        // Bar 1 is scored only against a labelled tempo: ours may be an octave off, and half
        // a bar would then pass a bar 1 that is two beats wrong.
        if let (Some(bar1), Some(label_bpm)) = (label.bar1_s, label.bpm) {
            let beats_per_bar = label_beats_per_bar(label)
                .unwrap_or_else(|| f64::from(grid.meter.beats_per_bar.max(1)));
            let bar = beats_per_bar * 60.0 / label_bpm;
            let ours = grid.anchor.to_seconds(record.spec.sample_rate).0;
            let d = ours - bar1;
            let wrapped = d - bar * (d / bar).round();
            row.bar1_diff_ms = Some(wrapped * 1000.0);
            row.bar1_ok = Some(wrapped.abs() * 1000.0 <= BAR1_TOLERANCE_MS);
            tally(&mut summary.bar1, row.bar1_ok);
        }
        rows.push(row);
    }
    (rows, summary)
}

/// Pulses per bar of a label: the sum of its grouping, else its meter's numerator.
fn label_beats_per_bar(label: &Label) -> Option<f64> {
    if let Some(grouping) = &label.grouping
        && !grouping.contains(&0)
    {
        return Some(grouping.iter().map(|&g| f64::from(g)).sum());
    }
    let meter = label.meter.as_deref()?;
    let (n, _) = meter.split_once('/')?;
    n.trim().parse::<f64>().ok().filter(|n| *n > 0.0)
}

fn tally(count: &mut (usize, usize), ok: Option<bool>) {
    if let Some(ok) = ok {
        count.1 += 1;
        if ok {
            count.0 += 1;
        }
    }
}

/// `9/8 · 2+2+2+3`-style text, as the grid's meter prints.
fn meter_text(meter: Option<&str>, grouping: Option<&[u8]>) -> Option<String> {
    let meter = meter?;
    match grouping {
        Some(g) if g.len() > 1 && g.iter().any(|&x| x > 1) => {
            let groups: Vec<String> = g.iter().map(u8::to_string).collect();
            Some(format!("{meter} · {}", groups.join("+")))
        }
        _ => Some(meter.to_owned()),
    }
}

/// Text table and summary.
pub fn write_text(
    rows: &[Scored],
    s: &Summary,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let mark = |ok: Option<bool>| match ok {
        Some(true) => "ok",
        Some(false) => "MISS",
        None => "-",
    };
    for r in rows {
        if let Some(why) = &r.skipped
            && r.bpm.is_none()
        {
            writeln!(out, "{:<48} skipped: {why}", truncate(&r.file, 48))?;
            continue;
        }
        writeln!(
            out,
            "{:<48} bpm {:>7} / {:>8} {:<4}  meter {:>14} / {:<14} {:<4}  bar1 {:>7} {:<4}  I {:>6} {}{}",
            truncate(&r.file, 48),
            r.bpm_label.map_or("-".into(), |b| format!("{b:.2}")),
            r.bpm.map_or("-".into(), |b| format!("{b:.3}")),
            if r.bpm_exact == Some(true) {
                "ok"
            } else if r.bpm_after_flip == Some(true) {
                "flip"
            } else {
                mark(r.bpm_exact)
            },
            r.meter_label.as_deref().unwrap_or("-"),
            r.meter.as_deref().unwrap_or("-"),
            mark(r.meter_ok),
            r.bar1_diff_ms.map_or("-".into(), |d| format!("{d:+.1}ms")),
            mark(r.bar1_ok),
            r.loudness_diff_lu
                .map_or("-".into(), |d| format!("{d:+.2}")),
            mark(r.loudness_ok),
            if r.fit == Some(GridFit::Start) {
                "  (start fit)"
            } else {
                ""
            },
        )?;
    }
    writeln!(out)?;
    writeln!(out, "analysed {}/{}", s.analysed, s.rows)?;
    writeln!(
        out,
        "bpm within {BPM_TOLERANCE}: {}/{}",
        s.bpm_exact.0, s.bpm_exact.1
    )?;
    writeln!(
        out,
        "bpm within {BPM_TOLERANCE} after at most one octave flip: {}/{}",
        s.bpm_after_flip.0, s.bpm_after_flip.1
    )?;
    writeln!(out, "meter and grouping: {}/{}", s.meter.0, s.meter.1)?;
    writeln!(
        out,
        "bar 1 within {BAR1_TOLERANCE_MS} ms: {}/{}",
        s.bar1.0, s.bar1.1
    )?;
    writeln!(
        out,
        "integrated within {LOUDNESS_TOLERANCE_LU} LU of ffmpeg: {}/{}",
        s.loudness.0, s.loudness.1
    )
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        s.chars().take(n - 1).chain(std::iter::once('…')).collect()
    }
}

#[cfg(test)]
mod tests;
