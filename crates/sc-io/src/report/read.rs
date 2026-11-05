//! Reading back a report SoundCheck wrote, so a later batch into the same folder can keep its rows:
//! RFC 4180 fields, the formula guard taken off again (the writer puts it back), numbers parsed
//! from their fixed format, so a row that is kept comes out with the same bytes.

use sc_core::{Bpm, Seconds};

use super::{BOM, COLUMNS, FORMULA_STARTS, ReportRow};

/// The records of `text`: RFC 4180, comma-separated, CRLF or LF line ends; `None` for one cut short
/// inside a quoted field.
fn records(text: &str) -> Option<Vec<Vec<String>>> {
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
    if quoted {
        return None;
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Some(rows)
}

/// A text field as it was before the writer guarded it against spreadsheet formulas.
fn text(field: &str) -> String {
    match field.strip_prefix('\'') {
        Some(rest) if rest.starts_with(FORMULA_STARTS) => rest.to_owned(),
        _ => field.to_owned(),
    }
}

/// A number field: `Ok(None)` when empty, `Err` when it is not a number.
fn number(field: &str) -> Result<Option<f64>, std::num::ParseFloatError> {
    if field.is_empty() {
        return Ok(None);
    }
    field.parse().map(Some)
}

/// The rows of the report `bytes`, in order; `None` when it is not readable whole as one with
/// today's columns: a row cut short, a number that is none. A caller must not take that for an
/// empty report: writing over it would lose what it says.
#[must_use]
pub fn read_rows(bytes: &[u8]) -> Option<Vec<ReportRow>> {
    let text_of = std::str::from_utf8(bytes.strip_prefix(BOM)?).ok()?;
    let mut records = records(text_of)?.into_iter();
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
                gain_db: number(&gain).ok()?,
                cut: number(&cut).ok()?.map(Seconds),
                bpm: number(&bpm).ok()?.map(Bpm),
                bar1: number(&bar1).ok()?.map(Seconds),
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
