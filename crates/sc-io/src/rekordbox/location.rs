//! `Location`: a track's file as a URI. The rekordbox format ("XML file format for playlists
//! sharing") asks for a URI and expects media under `file://localhost/`, so a location is
//! `file://localhost` followed by the absolute path with every byte percent-encoded except the
//! unreserved characters of RFC 3986 (letters, digits, `-`, `.`, `_`, `~`) and `/`: names with
//! spaces, `&`, `#`, `%` or non-ASCII letters (Turkish `ş`, `İ`, `ı` ...) survive any URI
//! parser.
//!
//! The bytes are the path's own: no Unicode normalisation is applied here. Callers pass the
//! path as the file system stores its names ([`crate::txn::resolve_file`]): a file named in
//! decomposed form on disk (as macOS's HFS+ and many Cocoa apps write names) keeps that form,
//! because macOS volumes open either spelling but rekordbox matches an existing collection entry
//! by the location string it recorded from the file system.

use std::path::{Path, PathBuf};

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
