//! Refusals found by looking at a request's files together, before anything is written: the
//! same file given twice (under any spelling: `T.wav`, `./T.wav`, the absolute path, another
//! case or Unicode form on macOS), and two files a copy into one folder would write under the
//! same name.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sc_core::Error;
use sc_io::txn::resolve_file;
use unicode_normalization::UnicodeNormalization;

use super::Place;

/// For each of `files`, in order, the refusal it gets before any file is written: a later
/// mention of a file already listed ([`Error::ListedTwice`]) or, with [`Place::Folder`], a
/// file whose output name an earlier one takes ([`Error::SameOutputName`]; names compared
/// ignoring case and Unicode form, as the default macOS volumes do). The first mention is
/// processed. A file that does not resolve gets no refusal here: processing it reports why.
#[must_use]
pub fn check_inputs(files: &[PathBuf], place: &Place) -> Vec<Option<Error>> {
    let mut seen: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    files
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let resolved = resolve_file(path).ok()?;
            if let Some(&first) = seen.get(&resolved) {
                return Some(Error::ListedTwice {
                    path: path.clone(),
                    first: files[first].clone(),
                });
            }
            seen.insert(resolved, i);
            let Place::Folder(dir) = place else {
                return None;
            };
            let name = path.file_name()?;
            let key: String = name
                .to_string_lossy()
                .nfc()
                .collect::<String>()
                .to_lowercase();
            if let Some(&other) = names.get(&key) {
                return Some(Error::SameOutputName {
                    path: path.clone(),
                    other: files[other].clone(),
                    output: dir.join(name),
                });
            }
            names.insert(key, i);
            None
        })
        .collect()
}

#[cfg(test)]
mod tests;
