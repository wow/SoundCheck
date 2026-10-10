//! The tag names callers use, the same for every container, and what each becomes in an
//! `ID3v2` tag (WAV, AIFF) or a Vorbis comment (FLAC).
//!
//! | Name | `ID3v2` | Vorbis comment |
//! |---|---|---|
//! | `BPM` | `TBPM` (the value rounded to an integer, as ID3v2.3 requires) and `TXXX:BPM` (the value as given) | `BPM` |
//! | `INITIALKEY` | `TKEY` | `INITIALKEY` |
//! | any other `NAME` | `TXXX:NAME` | `NAME` |
//!
//! [`Tag::parse`] reads `NAME=VALUE`; [`check_tags`] checks the names. A name is upper-case
//! letters, digits and `_` (`REPLAYGAIN_TRACK_GAIN`, `SOUNDCHECK`).
//! Container-specific labels are refused rather than written somewhere they mean nothing: a
//! label with `:` (`TXXX:BPM`) and an ID3 frame id ([`ID3_FRAME_IDS`]: `TBPM`, `TKEY`, ...).
//! Other four-letter names (`MOOD`, `YEAR`, `DATE`) are neutral names like any other.

pub use sc_core::Tag;
use sc_core::{Error, Result, TagEdit};
use sc_io::txn::TagFamily;

fn invalid(msg: String) -> Error {
    Error::InvalidArgument(msg)
}

/// The frame ids declared in ID3v2.3.0 section 4 and ID3v2.4.0-frames section 4 (their union),
/// plus the non-standard ones iTunes writes (`TCMP`, `TSO2`, `TSOC`, `GRP1`, `MVNM`, `MVIN`).
/// Sorted, for a binary search.
pub const ID3_FRAME_IDS: [&str; 98] = [
    "AENC", "APIC", "ASPI", "COMM", "COMR", "ENCR", "EQU2", "EQUA", "ETCO", "GEOB", "GRID", "GRP1",
    "IPLS", "LINK", "MCDI", "MLLT", "MVIN", "MVNM", "OWNE", "PCNT", "POPM", "POSS", "PRIV", "RBUF",
    "RVA2", "RVAD", "RVRB", "SEEK", "SIGN", "SYLT", "SYTC", "TALB", "TBPM", "TCMP", "TCOM", "TCON",
    "TCOP", "TDAT", "TDEN", "TDLY", "TDOR", "TDRC", "TDRL", "TDTG", "TENC", "TEXT", "TFLT", "TIME",
    "TIPL", "TIT1", "TIT2", "TIT3", "TKEY", "TLAN", "TLEN", "TMCL", "TMED", "TMOO", "TOAL", "TOFN",
    "TOLY", "TOPE", "TORY", "TOWN", "TPE1", "TPE2", "TPE3", "TPE4", "TPOS", "TPRO", "TPUB", "TRCK",
    "TRDA", "TRSN", "TRSO", "TSIZ", "TSO2", "TSOA", "TSOC", "TSOP", "TSOT", "TSRC", "TSSE", "TSST",
    "TXXX", "TYER", "UFID", "USER", "USLT", "WCOM", "WCOP", "WOAF", "WOAR", "WOAS", "WORS", "WPAY",
    "WPUB", "WXXX",
];

/// Whether `name` is an ID3 frame id (see [`ID3_FRAME_IDS`]).
fn is_frame_id(name: &str) -> bool {
    ID3_FRAME_IDS.binary_search(&name).is_ok()
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
        if is_frame_id(name) {
            return Err(invalid(format!(
                "tag {name} is an ID3 frame id; use a neutral name (BPM, INITIALKEY, or \
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
