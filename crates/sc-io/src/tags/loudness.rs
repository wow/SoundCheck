//! Which tag items hold a loudness value that a level change makes wrong.
//!
//! - Replay Gain 2.0: ID3 `TXXX:REPLAYGAIN_*` and Vorbis `REPLAYGAIN_*` (track and album gain
//!   and peak).
//! - EBU R 128 gains as Opus taggers write them: Vorbis `R128_*`.
//! - ID3v2.4 relative volume adjustment: `RVA2`.
//! - iTunes Sound Check: ID3 `COMM` described `iTunNORM` (and the Vorbis field `ITUNNORM` some
//!   taggers copy it to); ten hexadecimal words of levels and peaks.
//! - Serato's analysis (`Serato Autotags` GEOB, `SERATO_AUTOGAIN`/`SERATO_AUTOTAGS` Vorbis
//!   fields): tempo, auto gain and gain as text; its gain values describe the old level.
//!
//! An export replaces the track Replay Gain items it writes; the rest are reported and carried
//! unchanged. Gapless data (`iTunSMPB`) is not loudness and is not listed.

use crate::id3::FrameRef;

/// Whether `name` starts with `prefix`, ASCII case-insensitively.
fn starts_with_ignore_case(name: &[u8], prefix: &[u8]) -> bool {
    name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The label of an ID3 frame holding a loudness value, as reports show it (`TXXX:<description>`,
/// `RVA2`, `COMM:<description>`, `GEOB:<description>`, the description as written); `None` for
/// any other frame or one whose description cannot be read.
#[must_use]
pub fn id3_loudness_label(frame: &FrameRef) -> Option<String> {
    if &frame.id == b"RVA2" {
        return Some("RVA2".to_owned());
    }
    let is_loudness: fn(&str) -> bool = match &frame.id {
        b"TXXX" => |d| starts_with_ignore_case(d.as_bytes(), b"REPLAYGAIN_"),
        b"COMM" => |d| d.eq_ignore_ascii_case("iTunNORM"),
        b"GEOB" => |d| d.eq_ignore_ascii_case("Serato Autotags"),
        _ => return None,
    };
    let readings = [
        frame.description.as_deref(),
        frame.description_be.as_deref(),
    ];
    let desc = readings.into_iter().flatten().find(|d| is_loudness(d))?;
    Some(format!("{}:{desc}", String::from_utf8_lossy(&frame.id)))
}

/// Whether a Vorbis comment field name holds a loudness value: `REPLAYGAIN_*`, `R128_*`,
/// `ITUNNORM`, `SERATO_AUTOGAIN`, `SERATO_AUTOTAGS` (any case).
#[must_use]
pub fn is_vorbis_loudness(name: &[u8]) -> bool {
    starts_with_ignore_case(name, b"REPLAYGAIN_")
        || starts_with_ignore_case(name, b"R128_")
        || [&b"ITUNNORM"[..], b"SERATO_AUTOGAIN", b"SERATO_AUTOTAGS"]
            .iter()
            .any(|n| name.eq_ignore_ascii_case(n))
}
