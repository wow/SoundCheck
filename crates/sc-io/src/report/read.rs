//! Reading back a report SoundCheck wrote, so a later batch into the same folder can keep its rows:
//! RFC 4180 fields, the formula guard taken off again (the writer puts it back), numbers parsed
//! from their fixed format, so a row that is kept comes out with the same bytes.

use sc_core::{Bpm, Seconds};

use super::{BOM, COLUMNS, FORMULA_STARTS, ReportRow};

/// The records of `text`: RFC 4180, comma-separated, CRLF or LF line ends.
fn records(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// A text field as it was before the writer guarded it against spreadsheet formulas.
fn text(field: &str) -> String {
    match field.strip_prefix('\'') {
        Some(rest) if rest.starts_with(FORMULA_STARTS) => rest.to_owned(),
        _ => field.to_owned(),
    }
}

fn number(field: &str) -> Option<f64> {
    field.parse().ok()
}

/// The rows of the report `bytes`, in order; `None` when it is not one with today's columns.
#[must_use]
pub fn read_rows(bytes: &[u8]) -> Option<Vec<ReportRow>> {
    let text_of = std::str::from_utf8(bytes.strip_prefix(BOM)?).ok()?;
    let mut records = records(text_of).into_iter();
    if records.next()? != COLUMNS {
        return None;
    }
    records
        .map(|r| {
            let [file, mode, action, gain, cut, bpm, bar1, grid, check, notes]: [String; 10] =
                r.try_into().ok()?;
            let notes = text(&notes);
            let check = text(&check);
            Some(ReportRow {
                file: text(&file),
                mode: text(&mode),
                action: text(&action),
                gain_db: number(&gain),
                cut: number(&cut).map(Seconds),
                bpm: number(&bpm).map(Bpm),
                bar1: number(&bar1).map(Seconds),
                grid: text(&grid),
                grid_check: (!check.is_empty()).then_some(check),
                notes: if notes.is_empty() {
                    Vec::new()
                } else {
                    vec![notes]
                },
            })
        })
        .collect()
}
