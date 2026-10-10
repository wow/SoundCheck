//! The batch's grid report, `grid-report.csv`: one row per file, in the order the files were
//! given, with what the export did and the grid it carried.
//!
//! The format is RFC 4180: comma-separated, CRLF line ends, a header row, a field quoted when it
//! holds a comma, a quote, a line break or a leading or trailing space, quotes doubled inside.
//! The text is UTF-8 with a byte-order mark: spreadsheet apps (Excel in particular) read a CSV
//! without one in the system's legacy code page, which garbles Turkish and other non-ASCII
//! names; with it they read UTF-8. A text field starting with `=`, `+`, `-`, `@`, a tab or a
//! carriage return gets a leading `'` (OWASP, "CSV Injection") so a spreadsheet does not run a
//! file name or note as a formula; the numeric columns are written as plain numbers, so a
//! negative gain stays a number. Numbers use a dot and fixed decimals whatever the locale: gain in dB with a sign
//! and two decimals, the cut and bar 1 in seconds with three, the tempo with two.
//!
//! Columns: `file` (as given), `mode` (`prepare`, `library`, or empty for a file only read),
//! `action` (what happened to it), `gain_db`, `cut_s`, `bpm` (as written), `bar1_s` (bar 1 in
//! the file as it is now), `grid` (what the rekordbox XML carries for it, or why not),
//! `grid_check` (the check of the exported grid, when it ran), `notes` (`; `-separated).

use std::io::Write;

use sc_core::export::positive_zero;
use sc_core::{Bpm, Seconds};

/// File name of a batch's grid report.
pub const REPORT_FILE_NAME: &str = "grid-report.csv";

/// The header row.
pub const COLUMNS: [&str; 10] = [
    "file",
    "mode",
    "action",
    "gain_db",
    "cut_s",
    "bpm",
    "bar1_s",
    "grid",
    "grid_check",
    "notes",
];

/// Leading characters a spreadsheet reads as the start of a formula (OWASP, "CSV Injection"):
/// a text field starting with one gets a leading `'`.
const FORMULA_STARTS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// The UTF-8 byte-order mark the report starts with.
pub const BOM: &[u8] = b"\xEF\xBB\xBF";

/// One row of the report. Empty values are written as empty fields.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReportRow {
    /// The file as given.
    pub file: String,
    /// `prepare`, `library`, or empty.
    pub mode: String,
    /// What happened to the file.
    pub action: String,
    /// Gain applied, dB.
    pub gain_db: Option<f64>,
    /// Audio cut from the start.
    pub cut: Option<Seconds>,
    /// The tempo written (two decimals).
    pub bpm: Option<Bpm>,
    /// Bar 1 in the file as it is now.
    pub bar1: Option<Seconds>,
    /// What the rekordbox XML carries for the file's grid, or why it carries none.
    pub grid: String,
    /// The check of the exported grid; empty until it runs.
    pub grid_check: Option<String>,
    /// Anything else the user should know.
    pub notes: Vec<String>,
}

/// Whether `head` (the first bytes of a file) starts like a report SoundCheck wrote: the
/// byte-order mark and the header row.
#[must_use]
pub fn is_soundcheck_report(head: &[u8]) -> bool {
    head.strip_prefix(BOM)
        .is_some_and(|rest| rest.starts_with(COLUMNS.join(",").as_bytes()))
}

/// The report of `rows`, in the order given.
#[must_use]
pub fn csv_bytes(rows: &[ReportRow]) -> Vec<u8> {
    let mut out = Vec::with_capacity(128 + 160 * rows.len());
    // Writing into a Vec cannot fail.
    let written = write_csv(rows, &mut out);
    debug_assert!(written.is_ok(), "writing into memory failed: {written:?}");
    out
}

/// Writes the report of `rows` (see [`csv_bytes`]) to `out`.
///
/// # Errors
/// Whatever `out` returns.
pub fn write_csv(rows: &[ReportRow], out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(BOM)?;
    write_record(out, COLUMNS.iter().map(|c| Field::Number((*c).to_owned())))?;
    for row in rows {
        let number = |v: Option<String>| Field::Number(v.unwrap_or_default());
        write_record(
            out,
            [
                Field::Text(row.file.clone()),
                Field::Text(row.mode.clone()),
                Field::Text(row.action.clone()),
                number(row.gain_db.map(|g| format!("{:+.2}", positive_zero(g)))),
                number(row.cut.map(|c| format!("{:.3}", c.0))),
                number(row.bpm.map(|b| format!("{:.2}", b.written().0))),
                number(row.bar1.map(|b| format!("{:.3}", b.0))),
                Field::Text(row.grid.clone()),
                Field::Text(row.grid_check.clone().unwrap_or_default()),
                Field::Text(row.notes.join("; ")),
            ],
        )?;
    }
    Ok(())
}

/// A field: text (guarded against formulas) or a number we formatted.
enum Field {
    Text(String),
    Number(String),
}

fn write_record(
    out: &mut impl Write,
    fields: impl IntoIterator<Item = Field>,
) -> std::io::Result<()> {
    for (i, field) in fields.into_iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        let value = match field {
            Field::Text(s) if s.starts_with(FORMULA_STARTS) => format!("'{s}"),
            Field::Text(s) | Field::Number(s) => s,
        };
        out.write_all(quoted(&value).as_bytes())?;
    }
    out.write_all(b"\r\n")
}

/// `value` as an RFC 4180 field: quoted (inner quotes doubled) when it holds a comma, a quote,
/// a line break, or a leading or trailing space or tab.
fn quoted(value: &str) -> std::borrow::Cow<'_, str> {
    let needs = value.contains([',', '"', '\r', '\n'])
        || value.starts_with([' ', '\t'])
        || value.ends_with([' ', '\t']);
    if needs {
        std::borrow::Cow::Owned(format!("\"{}\"", value.replace('"', "\"\"")))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

#[cfg(test)]
mod tests;
