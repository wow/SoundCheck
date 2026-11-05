//! Which files the running jobs and commands are working on, so two of them never work on one
//! file: a second export would fail and its failure would hide the first's row, and a revert
//! during an export would let that export plan from one version of a file and write another.
//!
//! Files are held by the path the file system gives them, so two spellings or rows of one file are
//! one hold. A [`Hold`] is released when it is dropped, so on every way out of a job: the end of
//! it, an error, a cancel, a panic.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use sc_core::ipc::{IpcError, IpcErrorKind};

/// What a job or command does with the files it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldKind {
    /// Reads them: an analysis, or the rekordbox XML command. Any number at once.
    Reading,
    /// Exports them to copies in a folder: they do not change, but a change of them is refused.
    ExportCopy,
    /// Exports them in place: they change.
    ExportInPlace,
    /// Puts a file's original back: it changes.
    Revert,
}

impl HoldKind {
    fn is_reading(self) -> bool {
        self == Self::Reading
    }

    /// What is said about a file held so.
    fn doing(self) -> &'static str {
        match self {
            Self::Reading => "being analysed",
            Self::ExportCopy | Self::ExportInPlace => "being exported",
            Self::Revert => "being reverted",
        }
    }
}

#[derive(Debug, Default)]
struct FileHolds {
    readers: u32,
    /// An export or a revert, of which one at a time holds a file.
    worker: Option<HoldKind>,
}

impl FileHolds {
    /// What refuses `kind` on this file, if anything: an export or a revert needs the file to
    /// itself; an analysis is refused only while the file changes or is being exported.
    fn refusal(&self, kind: HoldKind) -> Option<HoldKind> {
        if let Some(worker) = self.worker {
            return Some(worker);
        }
        (!kind.is_reading() && self.readers > 0).then_some(HoldKind::Reading)
    }
}

/// The files held at this moment.
#[derive(Debug, Default)]
pub(crate) struct Holds {
    files: Mutex<BTreeMap<PathBuf, FileHolds>>,
}

impl Holds {
    fn files(&self) -> MutexGuard<'_, BTreeMap<PathBuf, FileHolds>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds all of `files` (id and path) as `kind`, or none of them.
    ///
    /// # Errors
    /// `busy`, naming the first file another job or command holds in a way `kind` cannot share.
    pub(crate) fn acquire(
        self: &Arc<Self>,
        files: &[(u32, &Path)],
        kind: HoldKind,
    ) -> Result<Hold, IpcError> {
        let keys: Vec<(u32, PathBuf)> = files.iter().map(|(id, path)| (*id, key(path))).collect();
        let mut held = self.files();
        for (id, path) in &keys {
            if let Some(by) = held.get(path).and_then(|file| file.refusal(kind)) {
                return Err(busy(*id, path, by));
            }
        }
        let keys: BTreeSet<PathBuf> = keys.into_iter().map(|(_, path)| path).collect();
        for path in &keys {
            let file = held.entry(path.clone()).or_default();
            if kind.is_reading() {
                file.readers += 1;
            } else {
                file.worker = Some(kind);
            }
        }
        drop(held);
        Ok(Hold {
            holds: Arc::clone(self),
            keys,
            kind,
        })
    }

    /// Refuses when the file is changing: exported in place or reverted. What a grid view opened
    /// now would show, and save, is about a version that is going away.
    ///
    /// # Errors
    /// `busy`, naming the file.
    pub(crate) fn refuse_changing(&self, file_id: u32, path: &Path) -> Result<(), IpcError> {
        let path = key(path);
        let worker = self.files().get(&path).and_then(|file| file.worker);
        match worker {
            Some(kind @ (HoldKind::ExportInPlace | HoldKind::Revert)) => {
                Err(busy(file_id, &path, kind))
            }
            _ => Ok(()),
        }
    }
}

/// The file's own name for it when it resolves, else the path as given.
fn key(path: &Path) -> PathBuf {
    sc_io::txn::resolve_file(path).unwrap_or_else(|_| path.to_path_buf())
}

fn busy(file_id: u32, path: &Path, by: HoldKind) -> IpcError {
    IpcError {
        kind: IpcErrorKind::Busy,
        message: format!(
            "{} is {}; try again when that has finished",
            path.display(),
            by.doing()
        ),
        file_id: Some(file_id),
    }
}

/// Files held until this is dropped.
#[derive(Debug)]
pub(crate) struct Hold {
    holds: Arc<Holds>,
    keys: BTreeSet<PathBuf>,
    kind: HoldKind,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut held = self.holds.files();
        for path in &self.keys {
            if let Some(file) = held.get_mut(path) {
                if self.kind.is_reading() {
                    file.readers = file.readers.saturating_sub(1);
                } else {
                    file.worker = None;
                }
                if file.readers == 0 && file.worker.is_none() {
                    held.remove(path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
