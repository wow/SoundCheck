//! File-system helpers for the transaction: durable syncs, streamed hashed copies, temp names,
//! no-replace renames, hex hashes and UTC dates.
//!
//! Durability follows the platform documentation: on Apple systems `File::sync_all` asks for
//! `F_FULLFSYNC` (fcntl(2): the drive flushes its cache too); a volume that refuses it gets a
//! plain `fsync(2)` instead, as SQLite's `full_fsync` does. After a rename the folder is synced
//! too (POSIX rename durability), and an error there is only logged.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sc_core::{Error, Result};

/// Bytes read per call when copying or hashing a file.
pub(crate) const COPY_BUFFER_BYTES: usize = 1 << 20;

/// What every temp file SoundCheck creates has in its name.
pub const TEMP_MARKER: &str = ".soundcheck-tmp-";

/// Longest part of the original name kept in a temp name, bytes (file names are limited to 255
/// bytes, and the temp name adds a prefix and the marker with the transaction id).
const TEMP_NAME_KEEP_BYTES: usize = 160;

/// [`Error::Io`] naming `path`.
pub(crate) fn io_err(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Flushes `file` to stable storage: `F_FULLFSYNC` where the volume supports it (std's
/// `sync_all` on Apple systems), else a plain `fsync`.
///
/// # Errors
/// [`Error::Io`] naming `path` when both fail.
pub(crate) fn sync_file(file: &File, path: &Path) -> Result<()> {
    match file.sync_all() {
        Ok(()) => Ok(()),
        Err(full) => match rustix::fs::fsync(file) {
            Ok(()) => {
                tracing::debug!(path = %path.display(), error = %full, "full sync refused; fsync used");
                Ok(())
            }
            Err(_) => Err(io_err(path, full)),
        },
    }
}

/// Opens `path` for writing (never creating it) and syncs it.
///
/// # Errors
/// [`Error::Io`] naming `path`.
pub(crate) fn sync_path(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| io_err(path, e))?;
    sync_file(&file, path)
}

/// Syncs the folder `dir` so a rename in it is durable; errors are only logged (some volumes
/// refuse to sync a folder, and the rename itself has already happened).
pub(crate) fn sync_dir(dir: &Path) {
    let synced = File::open(dir)
        .map_err(|e| io_err(dir, e))
        .and_then(|d| sync_file(&d, dir));
    if let Err(e) = synced {
        tracing::debug!(path = %dir.display(), error = %e, "folder sync failed (ignored)");
    }
}

/// Removes the file at `path` if it exists; whether it existed.
///
/// # Errors
/// [`Error::Io`] naming `path` for anything but "not found".
pub(crate) fn remove_if_exists(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_err(path, e)),
    }
}

/// Whether something (a file, a folder, a dangling link) exists at `path`.
pub(crate) fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// A BLAKE3 hash as 64 lowercase hex digits.
#[must_use]
pub fn hex(hash: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in hash {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Length and BLAKE3 of the file at `path`, read in [`COPY_BUFFER_BYTES`] blocks.
///
/// # Errors
/// [`Error::Io`] naming `path`.
pub(crate) fn hash_file(path: &Path) -> Result<(u64, [u8; 32])> {
    let mut file = File::open(path).map_err(|e| io_err(path, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0_u8; COPY_BUFFER_BYTES];
    let mut len = 0_u64;
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(path, e)),
        };
        hasher.update(&buf[..n]);
        len += n as u64;
    }
    Ok((len, *hasher.finalize().as_bytes()))
}

/// Copies the file `src` into the new file `dst` (created, never overwritten), hashing the
/// bytes as they pass, and syncs `dst`. Memory is bounded by one buffer. On any error `dst` is
/// removed.
///
/// # Errors
/// [`Error::Io`] naming `src` when reading fails and `dst` when creating or writing fails
/// (`dst` existing included).
pub(crate) fn copy_new_hashed(src: &Path, dst: &Path) -> Result<(u64, [u8; 32])> {
    let mut input = File::open(src).map_err(|e| io_err(src, e))?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| io_err(dst, e))?;
    let copied = copy_into(&mut input, src, &mut out, dst).and_then(|done| {
        sync_file(&out, dst)?;
        Ok(done)
    });
    if copied.is_err() {
        drop(out);
        if let Err(e) = remove_if_exists(dst) {
            tracing::warn!(path = %dst.display(), error = %e, "partial copy not removed");
        }
    }
    copied
}

fn copy_into(input: &mut File, src: &Path, out: &mut File, dst: &Path) -> Result<(u64, [u8; 32])> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0_u8; COPY_BUFFER_BYTES];
    let mut len = 0_u64;
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(src, e)),
        };
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| io_err(dst, e))?;
        len += n as u64;
    }
    Ok((len, *hasher.finalize().as_bytes()))
}

/// Renames `from` to `to` unless something exists at `to`, atomically where the volume can
/// (`renameatx_np(RENAME_EXCL)` on Apple systems, `renameat2(RENAME_NOREPLACE)` on Linux); a
/// volume without that falls back to a hard link and an unlink, which also never replaces.
///
/// # Errors
/// [`Error::AlreadyExists`] naming `to`; [`Error::Io`] naming `from` otherwise.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;
    match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => Err(Error::AlreadyExists {
            path: to.to_path_buf(),
        }),
        Err(e) if e == Errno::INVAL || e == Errno::NOSYS || e == Errno::NOTSUP => {
            tracing::debug!(path = %to.display(), "no-replace rename unsupported; linking");
            match std::fs::hard_link(from, to) {
                Ok(()) => remove_if_exists(from).map(|_| ()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(Error::AlreadyExists {
                        path: to.to_path_buf(),
                    })
                }
                Err(e) => Err(io_err(from, e)),
            }
        }
        Err(e) => Err(io_err(from, e.into())),
    }
}

/// A new transaction id: seconds since 1970, the process id and a counter, so ids never repeat
/// on one machine (they name temp and lock files, never anything hashed).
pub(crate) fn new_txn_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{secs:x}-{}-{n}", std::process::id())
}

/// `.<name>.soundcheck-tmp-<id>`: a hidden temp name next to `name` for transaction `id`, the
/// name cut (at a character boundary) to [`TEMP_NAME_KEEP_BYTES`].
pub(crate) fn temp_name(name: &OsStr, id: &str) -> OsString {
    let name = name.to_string_lossy();
    let mut end = name.len().min(TEMP_NAME_KEEP_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    OsString::from(format!(".{}{TEMP_MARKER}{id}", &name[..end]))
}

/// `path` with ` (n)` before its extension: `a/b.wav` -> `a/b (2).wav`.
pub(crate) fn numbered(path: &Path, n: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(OsStr::to_string_lossy)
        .unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day) (H. Hinnant,
/// "chrono-Compatible Low-Level Date Algorithms", `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // month is 1..=12 and day 1..=31 by construction.
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

/// Seconds since 1970 of `t` (0 before 1970).
fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The UTC date of `t` as `yyyy-mm-dd`.
pub(crate) fn utc_date(t: SystemTime) -> String {
    let (y, m, d) = civil_from_days(unix_secs(t).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `t` as an RFC 3339 UTC timestamp with seconds: `yyyy-mm-ddThh:mm:ssZ`.
pub(crate) fn utc_timestamp(t: SystemTime) -> String {
    let secs = unix_secs(t);
    let s = secs.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        utc_date(t),
        s / 3600,
        (s / 60) % 60,
        s % 60
    )
}

#[cfg(test)]
mod tests;
