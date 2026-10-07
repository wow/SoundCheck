//! Checks made before a transaction writes anything.
//!
//! In place (and for an undo), in this order: the path is a regular file and not a symbolic
//! link; it is not on a rekordbox USB export; it is not Finder-locked; it has no other hard
//! link; it has no access control list; it and its folder are writable (`access(2)`, and the
//! folder not locked). Writing a copy to a folder only reads the source, so of these only the
//! rekordbox check applies, to the destination folder. Then the container decides the render
//! (WAV, RF64, AIFF, AIFF-C: the IFF render; FLAC: the FLAC render; anything else is refused)
//! and its header gives an upper bound of the output's size, and every volume written to must
//! keep [`SPACE_MARGIN_BYTES`] free after the write.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use sc_core::{Error, InPlaceRefusal, RenderRequest, Result};

use super::fsx::io_err;
use super::meta::{effective_uid, has_acl, is_locked, owned_by_other};
use super::volume::{Volume, VolumeProvider};
use crate::{flac, iff};

/// Free space every volume written to keeps after the write, bytes (64 MiB).
pub const SPACE_MARGIN_BYTES: u64 = 64 << 20;

/// Which render a file takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Container {
    /// WAV/RF64 (`wave`) or AIFF/AIFF-C.
    Iff {
        /// Whether the output is RIFF/WAVE.
        wave: bool,
    },
    /// FLAC.
    Flac,
}

/// A source file that passed its checks.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    /// Absolute path, its folder resolved (the file itself is not a link).
    pub path: PathBuf,
    /// Its file name.
    pub name: OsString,
    /// Which render it takes.
    pub container: Container,
    /// Upper bound of the output's size, bytes.
    pub output_estimate: u64,
}

fn refuse(path: &Path, reason: InPlaceRefusal) -> Error {
    Error::InPlaceRefused {
        path: path.to_path_buf(),
        reason,
    }
}

/// The path the file system itself holds for the existing file or folder `path`: absolute,
/// links followed, and every name as stored on disk (on macOS read back from the open file,
/// `F_GETPATH`, so `track.wav` reached as `Track.wav`, or a decomposed `Café` reached through a
/// precomposed one, gives one path).
///
/// # Errors
/// [`Error::Io`] naming `path`.
pub(crate) fn real_path(path: &Path) -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStringExt;
        let file = std::fs::File::open(path).map_err(|e| io_err(path, e))?;
        let raw = rustix::fs::getpath(&file).map_err(|e| io_err(path, e.into()))?;
        Ok(PathBuf::from(OsString::from_vec(raw.into_bytes())))
    }
    #[cfg(not(target_os = "macos"))]
    {
        path.canonicalize().map_err(|e| io_err(path, e))
    }
}

/// The nearest existing folder at or above `path` (resolved), and the part below it.
pub(crate) fn nearest_existing(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| io_err(path, e))?
            .join(path)
    };
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    while !existing.is_dir() {
        rest.push(existing.file_name().unwrap_or_default().to_os_string());
        existing = existing.parent().ok_or_else(|| {
            Error::InvalidArgument(format!("{} has no existing folder", path.display()))
        })?;
    }
    let below: PathBuf = rest.iter().rev().collect();
    Ok((real_path(existing)?, below))
}

/// Whether `path` (resolved) lies in the backup root `root` (which may not exist yet). Library
/// scanners must skip these paths: the backups are SoundCheck's own copies.
///
/// # Errors
/// [`Error::Io`] when neither path resolves.
pub fn is_under_backup_root(path: &Path, root: &Path) -> Result<bool> {
    let (base, below) = nearest_existing(root)?;
    let root = base.join(below);
    let path = if let Ok(p) = real_path(path) {
        p
    } else {
        let (base, below) = nearest_existing(path)?;
        base.join(below)
    };
    Ok(path.starts_with(&root))
}

/// [`Error::InvalidArgument`] when `path` lies in the backup root.
pub(crate) fn check_outside_backups(path: &Path, root: &Path) -> Result<()> {
    if is_under_backup_root(path, root)? {
        return Err(Error::InvalidArgument(format!(
            "{} is inside the backup folder {}; SoundCheck does not process its own backups. \
             Copy the file out of it first",
            path.display(),
            root.display()
        )));
    }
    Ok(())
}

/// `path` as the file system holds it: `(full path, folder, file name)`. A symbolic link is
/// kept as a link in its resolved folder (so it can be refused in place); anything else is
/// [`real_path`]. Refuses a path that is not valid UTF-8 (the journal stores paths as text).
///
/// # Errors
/// [`Error::Io`] when the path does not resolve; [`Error::InvalidArgument`] for a path
/// without a file name or not valid UTF-8.
pub(crate) fn resolve(path: &Path) -> Result<(PathBuf, PathBuf, OsString)> {
    let link = std::fs::symlink_metadata(path)
        .map_err(|e| io_err(path, e))?
        .file_type()
        .is_symlink();
    let full = if link {
        let name = path
            .file_name()
            .ok_or_else(|| Error::InvalidArgument(format!("{} names no file", path.display())))?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        real_path(&parent)?.join(name)
    } else {
        real_path(path)?
    };
    let (Some(dir), Some(name)) = (full.parent(), full.file_name()) else {
        return Err(Error::InvalidArgument(format!(
            "{} names no file",
            path.display()
        )));
    };
    if full.to_str().is_none() {
        return Err(Error::InvalidArgument(format!(
            "{} is not a UTF-8 path",
            full.display()
        )));
    }
    Ok((full.clone(), dir.to_path_buf(), name.to_os_string()))
}

/// The in-place checks on `path` (resolved); returns the file's volume.
///
/// # Errors
/// [`Error::InPlaceRefused`], [`Error::RekordboxUsbExport`], [`Error::InvalidArgument`] for
/// something that is not a regular file, [`Error::Io`].
pub(crate) fn check_in_place(
    path: &Path,
    dir: &Path,
    volumes: &dyn VolumeProvider,
) -> Result<Volume> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| io_err(path, e))?;
    if meta.file_type().is_symlink() {
        return Err(refuse(path, InPlaceRefusal::Symlink));
    }
    if !meta.is_file() {
        return Err(Error::InvalidArgument(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    let volume = volumes.volume_of(dir)?;
    check_not_rekordbox(path, &volume)?;
    if is_locked(&meta) {
        return Err(refuse(path, InPlaceRefusal::FinderLocked));
    }
    if owned_by_other(meta.uid(), effective_uid()) {
        return Err(refuse(path, InPlaceRefusal::OwnedByOtherUser));
    }
    if meta.nlink() > 1 {
        return Err(refuse(
            path,
            InPlaceRefusal::HardLinked {
                links: meta.nlink(),
            },
        ));
    }
    if has_acl(path)? {
        return Err(refuse(path, InPlaceRefusal::HasAcl));
    }
    if rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_err() {
        return Err(refuse(path, InPlaceRefusal::ReadOnlyFile));
    }
    let dir_meta = std::fs::metadata(dir).map_err(|e| io_err(dir, e))?;
    let dir_writable = rustix::fs::access(
        dir,
        rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK,
    )
    .is_ok();
    if !dir_writable || is_locked(&dir_meta) {
        return Err(refuse(path, InPlaceRefusal::ReadOnlyFolder));
    }
    Ok(volume)
}

/// [`Error::RekordboxUsbExport`] when `volume` is a rekordbox USB export.
///
/// # Errors
/// As described.
pub(crate) fn check_not_rekordbox(path: &Path, volume: &Volume) -> Result<()> {
    if volume.is_rekordbox_export() {
        return Err(Error::RekordboxUsbExport {
            path: path.to_path_buf(),
            volume: volume.root.clone(),
        });
    }
    Ok(())
}

/// The container of the regular file `path` and an upper bound of its rendered size.
///
/// # Errors
/// [`Error::UnsupportedFormat`] for anything but WAV, RF64, AIFF, AIFF-C and FLAC; the header
/// readers' errors.
pub(crate) fn source(path: PathBuf, name: OsString, req: &RenderRequest) -> Result<Source> {
    let mut file = File::open(&path).map_err(|e| io_err(&path, e))?;
    let mut magic = [0_u8; 12];
    let got = read_up_to(&mut file, &mut magic).map_err(|e| io_err(&path, e))?;
    let magic = &magic[..got];
    let (container, output_estimate) = if is_iff(magic) {
        let header = iff::read_header(&mut file, &path)?;
        let f = &header.format;
        let bits = out_bytes(req, f.encoding.is_float() || f.valid_bits > 16);
        let audio = f.frames * u64::from(f.channels) * bits;
        let other = header_len(&file, &path)?.saturating_sub(f.data.end - f.data.start);
        (
            Container::Iff {
                wave: header.table.container.is_wave(),
            },
            other + audio + 64,
        )
    } else if magic.starts_with(&flac::MARKER) || magic.starts_with(b"ID3") {
        let layout = flac::read_layout(&mut file, &path)?;
        let info = &layout.streaminfo;
        let frames = match info.total_samples {
            // Unknown total: bounded by the source's own size.
            0 => layout.file_len,
            n => n,
        };
        let bits = out_bytes(req, info.bits > 16);
        // Verbatim frames at most, plus frame headers (well under 1 %), plus the metadata and
        // the tags around the stream.
        let audio = frames * u64::from(info.channels) * bits;
        let estimate = audio + audio / 100 + layout.frames_start + 65_536 + layout.file_len
            - layout.frames_limit();
        (Container::Flac, estimate)
    } else {
        return Err(Error::UnsupportedFormat {
            path,
            detail: "only WAV, RF64, AIFF, AIFF-C and FLAC files are written".into(),
        });
    };
    Ok(Source {
        path,
        name,
        container,
        output_estimate,
    })
}

fn is_iff(magic: &[u8]) -> bool {
    magic.len() == 12
        && ((matches!(&magic[..4], b"RIFF" | b"RF64" | b"BW64") && &magic[8..] == b"WAVE")
            || (&magic[..4] == b"FORM" && matches!(&magic[8..], b"AIFF" | b"AIFC")))
}

/// Bytes per output sample: the requested depth, else 3 for a deep or float source, else 2.
fn out_bytes(req: &RenderRequest, deep: bool) -> u64 {
    match req.bits {
        Some(b) => u64::from(b).div_ceil(8),
        None if deep => 3,
        None => 2,
    }
}

fn header_len(file: &File, path: &Path) -> Result<u64> {
    Ok(file.metadata().map_err(|e| io_err(path, e))?.len())
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match file.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

/// Checks that every volume in `needs` (a volume and the bytes written to it) keeps
/// [`SPACE_MARGIN_BYTES`] free; needs on the same volume add up.
///
/// # Errors
/// [`Error::NoSpace`] naming the first volume short of space.
pub(crate) fn check_space(needs: &[(&Volume, u64)]) -> Result<()> {
    let mut seen: Vec<u64> = Vec::new();
    for (volume, _) in needs {
        if seen.contains(&volume.id) {
            continue;
        }
        seen.push(volume.id);
        let total: u64 = needs
            .iter()
            .filter(|(v, _)| v.id == volume.id)
            .map(|(_, n)| *n)
            .sum();
        let needed = total.saturating_add(SPACE_MARGIN_BYTES);
        if volume.free_bytes < needed {
            return Err(Error::NoSpace {
                volume: volume.root.clone(),
                needed_bytes: needed,
                free_bytes: volume.free_bytes,
            });
        }
    }
    Ok(())
}
