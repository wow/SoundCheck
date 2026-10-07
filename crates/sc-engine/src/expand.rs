//! Turning what the user dropped into rows: walk folders, keep audio files, drop duplicates, and
//! read each file's headers and tags in parallel so every row has its title and spec at once.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use sc_core::ipc::FileInfo;
use sc_core::plan::Codec;
use sc_io::probe;
use sc_io::txn::is_under_backup_root;
use unicode_normalization::UnicodeNormalization;

/// File extensions SoundCheck opens, lower case.
pub const AUDIO_EXTENSIONS: [&str; 14] = [
    "wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "m4a", "mp4", "aac", "alac", "ogg", "oga",
    "opus",
];

/// Audio files under `paths`, in the order a file browser lists them: each dropped path in turn,
/// folders walked depth first with names in natural order ("Track 2" before "Track 10").
///
/// Hidden entries (see [`is_hidden`]) and symbolic links inside folders are skipped; a path
/// dropped directly is followed. A file reached twice (the same NFC path) appears once.
/// Nothing in `backup_root` (SoundCheck's own backups of the originals it changed) is listed,
/// whether dropped directly or reached through a folder above it.
#[must_use]
pub fn collect_audio_files(paths: &[PathBuf], backup_root: Option<&Path>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in paths {
        if in_backups(path, backup_root) {
            continue;
        }
        if path.is_dir() {
            walk(path, backup_root, &mut seen, &mut out);
        } else if is_audio(path) {
            push_unique(path.clone(), &mut seen, &mut out);
        }
    }
    out
}

/// Whether `path` lies in the backup root (false when either path cannot be resolved).
fn in_backups(path: &Path, backup_root: Option<&Path>) -> bool {
    let inside = backup_root.is_some_and(|root| is_under_backup_root(path, root).unwrap_or(false));
    if inside {
        tracing::debug!(path = %path.display(), "skipped: in the backup folder");
    }
    inside
}

/// Probes `files` on up to `workers` threads; the result is in the same order.
///
/// A file whose probe fails keeps the codec its name suggests and no other field.
#[must_use]
pub fn probe_all(files: &[PathBuf], workers: usize) -> Vec<FileInfo> {
    // Each row starts as its name suggests; a worker that panics leaves its rows like that.
    let mut infos: Vec<FileInfo> = files
        .iter()
        .map(|p| FileInfo {
            codec: Codec::from_path(p),
            ..FileInfo::default()
        })
        .collect();
    let next = AtomicUsize::new(0);
    let workers = workers.clamp(1, files.len().max(1));
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, AtomicOrdering::Relaxed);
                        let Some(path) = files.get(i) else { break };
                        done.push((i, probe(path)));
                    }
                    done
                })
            })
            .collect();
        for handle in handles {
            if let Ok(done) = handle.join() {
                for (i, info) in done {
                    infos[i] = info;
                }
            }
        }
    });
    infos
}

fn walk(
    dir: &Path,
    backup_root: Option<&Path>,
    seen: &mut HashSet<String>,
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| !is_hidden(&e.file_name().to_string_lossy()))
        .collect();
    entries.sort_by(|a, b| {
        natural_cmp(
            &a.file_name().to_string_lossy(),
            &b.file_name().to_string_lossy(),
        )
    });
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            // A folder outside the backup root holds only files outside it (links are not
            // followed), so the check is made once per folder.
            if !in_backups(&path, backup_root) {
                walk(&path, backup_root, seen, out);
            }
        } else if kind.is_file() && is_audio(&path) {
            push_unique(path, seen, out);
        }
    }
}

/// A dot-file or dot-folder: one leading `.` followed by something else (`.Trash`, `.DS_Store`,
/// macOS `._` resource files). Names that start with an ellipsis or two dots are titles, not
/// hidden entries: "...Baby One More Time", "...And Justice for All".
fn is_hidden(name: &str) -> bool {
    name.starts_with('.') && !name.starts_with("..")
}

fn push_unique(path: PathBuf, seen: &mut HashSet<String>, out: &mut Vec<PathBuf>) {
    let key: String = path.to_string_lossy().nfc().collect();
    if seen.insert(key) {
        out.push(path);
    }
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Case-insensitive comparison that orders runs of digits by value.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_number(&mut a);
                let nb = take_number(&mut b);
                // Compare by length, then lexically: no overflow on long digit runs.
                let order = na
                    .trim_start_matches('0')
                    .len()
                    .cmp(&nb.trim_start_matches('0').len())
                    .then_with(|| na.trim_start_matches('0').cmp(nb.trim_start_matches('0')));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
        digits.push(c);
        chars.next();
    }
    digits
}

#[cfg(test)]
mod tests;
