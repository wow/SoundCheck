//! `Location`: a track's file as a URI. The rekordbox format ("XML file format for playlists
//! sharing") asks for a URI and expects media under `file://localhost/`, so a location is
//! `file://localhost` followed by the absolute path with every byte percent-encoded except the
//! unreserved characters of RFC 3986 (letters, digits, `-`, `.`, `_`, `~`) and `/`: names with
//! spaces, `&`, `#`, `%` or non-ASCII letters (Turkish `ş`, `İ`, `ı` ...) survive any URI
//! parser.
//!
//! [`location`] writes the path's own bytes, with no Unicode normalisation; [`Speller`] gives
//! the path to write: every name as its folder lists it, the entry found by its inode. That is
//! the spelling the Finder, and rekordbox when it added the file, see: macOS volumes open a
//! file under any spelling that is canonically equivalent (and on most volumes under any case),
//! so the spelling typed or passed in can differ from the listing's, and asking the open file
//! for its path is no better (on FAT32 the system answers with whichever spelling first reached
//! the file since the volume was mounted). rekordbox matches an existing collection entry by
//! its location string, so another spelling would add the track a second time.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode, percent_encode};

/// The URI scheme and host every location starts with.
const PREFIX: &str = "file://localhost";

/// Bytes written as `%XX`: everything but RFC 3986 unreserved characters and the separator.
const ENCODED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// `path` as a rekordbox `Location`: `file://localhost/Users/dj/M%C3%BCzik/Track%201.wav`.
/// A relative path is written as if it started at the root (callers pass absolute paths).
#[must_use]
pub fn location(path: &Path) -> String {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut out = String::with_capacity(PREFIX.len() + bytes.len() * 2);
    out.push_str(PREFIX);
    if bytes.first() != Some(&b'/') {
        out.push('/');
    }
    out.extend(percent_encode(bytes, ENCODED));
    out
}

/// The path a `Location` names, or `None` when it does not start with `file://localhost/` or
/// does not decode to UTF-8 (the inverse of [`location`] for UTF-8 paths).
#[must_use]
pub fn decode_location(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix(PREFIX)?;
    if !rest.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(rest.as_bytes()).decode_utf8().ok()?;
    Some(PathBuf::from(decoded.into_owned()))
}

/// Spells paths as their folders list their names, reading each folder once.
#[derive(Debug, Default)]
pub struct Speller {
    /// Each folder's entry names, sorted; `None` when it could not be listed.
    listings: BTreeMap<PathBuf, Option<Vec<OsString>>>,
}

impl Speller {
    /// A speller that has read no folder yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `path` made absolute (`.` dropped, `..` taking the name before it away), each name
    /// replaced by its folder's entry for the same file: the entry whose name equals it
    /// ignoring Unicode normalisation and case and whose inode is the file's (the first such
    /// name in byte order). A name no entry matches, in a folder that cannot be listed, or on
    /// a system without inodes, stays as given; links are not followed.
    pub fn spell(&mut self, path: &Path) -> PathBuf {
        let Ok(absolute) = std::path::absolute(path) else {
            return path.to_path_buf();
        };
        let mut out = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => out.push(component),
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                Component::Normal(name) => {
                    let listed = self.listed_name(&out, name);
                    out.push(listed.as_deref().unwrap_or(name));
                }
            }
        }
        out
    }

    #[cfg(unix)]
    fn listed_name(&mut self, dir: &Path, name: &OsStr) -> Option<OsString> {
        use std::os::unix::fs::MetadataExt;
        let ino = std::fs::symlink_metadata(dir.join(name)).ok()?.ino();
        let names = self
            .listings
            .entry(dir.to_path_buf())
            .or_insert_with(|| list(dir))
            .as_ref()?;
        let want = fold(name);
        names
            .iter()
            .filter(|n| fold(n) == want)
            .find(|n| std::fs::symlink_metadata(dir.join(n)).is_ok_and(|m| m.ino() == ino))
            .cloned()
    }

    #[cfg(not(unix))]
    fn listed_name(&mut self, _dir: &Path, _name: &OsStr) -> Option<OsString> {
        None
    }
}

/// The names in `dir`, sorted by their bytes.
#[cfg(unix)]
fn list(dir: &Path) -> Option<Vec<OsString>> {
    let mut names: Vec<OsString> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    names.sort();
    Some(names)
}

/// `name` compared ignoring Unicode normalisation (NFC) and case.
#[cfg(unix)]
fn fold(name: &OsStr) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.to_string_lossy()
        .nfc()
        .collect::<String>()
        .to_lowercase()
}
