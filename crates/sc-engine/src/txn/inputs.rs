//! Refusals found by looking at a request's files together, before anything is written: the
//! same file given twice (under any spelling: `T.wav`, `./T.wav`, the absolute path, another
//! case or Unicode form on macOS), and two files a copy into one folder would write under the
//! same name.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sc_core::Error;
use sc_io::txn::{folder_folds_case, resolve_file};
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
    let folds = matches!(place, Place::Folder(dir) if folder_folds_case(dir));
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
            let key = name_key(&name.to_string_lossy(), folds);
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

/// The names files of a batch take in one folder, as the folder's volume compares them.
#[derive(Debug, Clone)]
pub struct FolderNames {
    folds: bool,
    taken: BTreeSet<String>,
}

impl FolderNames {
    /// No name taken yet in `dir`, which need not exist.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            folds: folder_folds_case(dir),
            taken: BTreeSet::new(),
        }
    }

    /// Takes `name`; `false` when an earlier file took it.
    pub fn claim(&mut self, name: &OsStr) -> bool {
        self.taken
            .insert(name_key(&name.to_string_lossy(), self.folds))
    }
}

/// How the folder compares a name: `folds` when `Track.wav` and `track.wav` are one file there.
/// Unicode forms are always one name: macOS volumes treat them so, and on any other volume two
/// spellings of one name are worth refusing.
fn name_key(name: &str, folds: bool) -> String {
    let nfc: String = name.nfc().collect();
    if folds { nfc.to_lowercase() } else { nfc }
}

#[cfg(test)]
mod tests;
