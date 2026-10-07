//! The tag names callers use, the same for every container, and what each becomes in an
//! `ID3v2` tag (WAV, AIFF) or a Vorbis comment (FLAC).
//!
//! | Name | `ID3v2` | Vorbis comment |
//! |---|---|---|
//! | `BPM` | `TBPM` (the value rounded to an integer, as ID3v2.3 requires) and `TXXX:BPM` (the value as given) | `BPM` |
//! | `INITIALKEY` | `TKEY` | `INITIALKEY` |
//! | any other `NAME` | `TXXX:NAME` | `NAME` |
//!
//! A name is upper-case letters, digits and `_` (`REPLAYGAIN_TRACK_GAIN`, `SOUNDCHECK`).
//! Container-specific labels are refused rather than written somewhere they mean nothing: a
//! label with `:` (`TXXX:BPM`) and a four-character name shaped like an ID3 frame id
//! (`TBPM`, `TKEY`; `[A-Z][A-Z0-9]{3}`).

use sc_core::{Error, Result, TagEdit};
use sc_io::txn::TagFamily;

/// A tag item to add or replace, by its container-neutral name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Tag {
    /// The name (see the module documentation).
    pub name: String,
    /// The text value.
    pub value: String,
}

impl Tag {
    /// Parses `NAME=VALUE` (split at the first `=`); the name is upper-cased.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] without `=` or a name; the name is checked by [`check_tags`].
    pub fn parse(text: &str) -> Result<Self> {
        let (name, value) = text
            .split_once('=')
            .ok_or_else(|| Error::InvalidArgument(format!("tag {text:?} is not NAME=VALUE")))?;
        if name.is_empty() {
            return Err(Error::InvalidArgument(format!("tag {text:?} has no name")));
        }
        Ok(Self {
            name: name.to_ascii_uppercase(),
            value: value.to_owned(),
        })
    }
}

fn invalid(msg: String) -> Error {
    Error::InvalidArgument(msg)
}

/// Whether `name` has the shape of an ID3v2.3/2.4 frame id.
fn looks_like_frame_id(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 4
        && b[0].is_ascii_uppercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Checks every tag: a valid neutral name, not given twice, a value without NUL, a `BPM` that is
/// a positive number.
///
/// # Errors
/// [`Error::InvalidArgument`] naming the first tag that fails and why.
pub fn check_tags(tags: &[Tag]) -> Result<()> {
    for (i, tag) in tags.iter().enumerate() {
        let name = tag.name.as_str();
        if name.contains(':') {
            return Err(invalid(format!(
                "tag {name} is a container label; use the neutral name ({} for {name})",
                name.rsplit(':').next().unwrap_or_default()
            )));
        }
        if looks_like_frame_id(name) {
            return Err(invalid(format!(
                "tag {name} looks like an ID3 frame id; use a neutral name (BPM, INITIALKEY, or \
                 another NAME, which becomes TXXX:NAME in ID3 and NAME in FLAC)"
            )));
        }
        let valid = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(invalid(format!(
                "tag name {name:?} must be 1 to 64 upper-case letters, digits or _"
            )));
        }
        if tags[..i].iter().any(|t| t.name == tag.name) {
            return Err(invalid(format!("tag {name} is given twice")));
        }
        if tag.value.contains('\0') {
            return Err(invalid(format!("tag {name}'s value holds a NUL character")));
        }
        if name == "BPM"
            && !tag
                .value
                .parse::<f64>()
                .is_ok_and(|v| v.is_finite() && v > 0.0)
        {
            return Err(invalid(format!(
                "tag BPM must be a positive number, not {:?}",
                tag.value
            )));
        }
    }
    Ok(())
}

/// What `tags` (checked by [`check_tags`]) become in a tag of `family`, in order.
#[must_use]
pub fn tag_edits(tags: &[Tag], family: TagFamily) -> Vec<TagEdit> {
    let edit = |label: &str, value: &str| TagEdit {
        label: label.to_owned(),
        value: value.to_owned(),
    };
    let mut out = Vec::new();
    for tag in tags {
        let (name, value) = (tag.name.as_str(), tag.value.as_str());
        match (family, name) {
            (TagFamily::Id3, "BPM") => {
                let bpm = value.parse::<f64>().unwrap_or_default().round();
                out.push(edit("TBPM", &format!("{bpm:.0}")));
                out.push(edit("TXXX:BPM", value));
            }
            (TagFamily::Id3, "INITIALKEY") => out.push(edit("TKEY", value)),
            (TagFamily::Id3, _) => out.push(edit(&format!("TXXX:{name}"), value)),
            (TagFamily::Vorbis, _) => out.push(edit(name, value)),
        }
    }
    out
}

#[cfg(test)]
mod tests;
