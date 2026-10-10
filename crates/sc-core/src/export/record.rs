//! The `SOUNDCHECK` tag: what SoundCheck did to a file, written by an export and read back so a
//! later run can see that a file was already processed.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{BatchMode, positive_zero};
use crate::plan::LoudnessMode;
use crate::units::{Bpm, Lufs, SampleIndex};

/// The record's format version.
pub const SOUNDCHECK_RECORD_VERSION: u32 = 1;

/// Longest record value [`SoundcheckRecord::parse`] reads, bytes; a longer tag is not a record.
pub const MAX_RECORD_BYTES: usize = 1024;

/// The level change a [`SoundcheckRecord`] records.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RecordGain {
    /// The statistic the gain aligned.
    pub stat: LoudnessMode,
    /// The target it aligned to.
    pub target: Lufs,
    /// The gain applied, dB.
    pub gain_db: f64,
}

/// The value of the `SOUNDCHECK` tag: what SoundCheck did to the file, as `key=value` pairs
/// joined by `;`, in this order:
/// `v=1;app=<version>;mode=prepare|library;stat=S-P95|I;target=<LUFS, 2 dp>;gain=<dB, signed,
/// 2 dp>;trim=<frames>;rate=<Hz>;bpm=<2 dp>;bar1=<samples>;src=<16 hex digits>`.
/// Grid only writes `gain=none` and no `stat` or `target` (no level was aligned). Positions are
/// sample counts at `rate`, as everywhere else: `trim` in the source, `bar1` (the first bar line)
/// in the exported audio. `bpm` and `bar1` are left out without a grid, and written as
/// `bpm=none;bar1=none` when the grid was not trusted (it needed review and the user had not
/// confirmed it), so the record never vouches for such a grid; `src` is left out without a
/// source hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SoundcheckRecord {
    /// The SoundCheck version that wrote it (no `;`).
    pub app: String,
    /// The batch mode.
    pub mode: BatchMode,
    /// The level change; `None` for grid only.
    pub gain: Option<RecordGain>,
    /// Frames removed from the start.
    #[ts(type = "number")]
    pub trim_frames: u64,
    /// The sample rate, Hz.
    pub sample_rate: u32,
    /// The grid's tempo at the meter's unit, as written ([`Bpm::written`]).
    pub bpm: Option<Bpm>,
    /// The first bar line of the exported audio.
    pub bar1: Option<SampleIndex>,
    /// The file has a grid, but it needed review and was not confirmed, so neither its tempo
    /// nor its bar 1 is recorded (`bpm=none;bar1=none`; `bpm` and `bar1` are then `None`).
    #[serde(default)]
    pub grid_withheld: bool,
    /// The first 8 bytes of the source file's BLAKE3 hash (16 hex digits in the tag).
    pub source_hash: Option<[u8; 8]>,
}

/// Why a tag value is not a record SoundCheck can read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// The value is longer than [`MAX_RECORD_BYTES`].
    #[error("the record has {0} bytes, at most {MAX_RECORD_BYTES}")]
    TooLong(usize),
    /// A record of another format version (`v`), written by a newer or older SoundCheck.
    #[error("record version {0}; this version reads version {SOUNDCHECK_RECORD_VERSION}")]
    UnsupportedVersion(String),
    /// A required key is missing.
    #[error("the record has no {0:?}")]
    Missing(&'static str),
    /// A key appears twice.
    #[error("the record has {0:?} twice")]
    Duplicate(String),
    /// A part without `=`.
    #[error("{0:?} is not a key=value pair")]
    NotAPair(String),
    /// A value that does not read as its key's type.
    #[error("{key}={value:?} is not valid")]
    Invalid {
        /// The key.
        key: &'static str,
        /// The value as written.
        value: String,
    },
}

impl SoundcheckRecord {
    /// The first 8 bytes of a BLAKE3 hash, as the record keeps them.
    #[must_use]
    pub fn hash_prefix(blake3: &[u8; 32]) -> [u8; 8] {
        let mut prefix = [0; 8];
        prefix.copy_from_slice(&blake3[..8]);
        prefix
    }

    /// The tag value (see the type's documentation).
    #[must_use]
    pub fn to_value(&self) -> String {
        let mut v = format!(
            "v={SOUNDCHECK_RECORD_VERSION};app={};mode={}",
            self.app,
            self.mode.as_str()
        );
        // Writing into a String cannot fail.
        match self.gain {
            Some(g) => {
                let _ = write!(
                    v,
                    ";stat={};target={:.2};gain={:+.2}",
                    stat_str(g.stat),
                    positive_zero(g.target.0),
                    positive_zero(g.gain_db)
                );
            }
            None => v.push_str(";gain=none"),
        }
        let _ = write!(v, ";trim={};rate={}", self.trim_frames, self.sample_rate);
        if self.grid_withheld {
            v.push_str(";bpm=none;bar1=none");
        } else {
            if let Some(bpm) = self.bpm {
                let _ = write!(v, ";bpm={:.2}", bpm.0);
            }
            if let Some(bar1) = self.bar1 {
                let _ = write!(v, ";bar1={}", bar1.0);
            }
        }
        if let Some(hash) = self.source_hash {
            v.push_str(";src=");
            for b in hash {
                let _ = write!(v, "{b:02x}");
            }
        }
        v
    }

    /// Reads a tag value written by [`SoundcheckRecord::to_value`]: `to_value(parse(v)) == v`
    /// for every value it writes. Keys may come in any order and unknown keys are ignored, so a
    /// later version 1 writer may add keys; surrounding white space of the whole value is
    /// ignored. Numbers are read as written (`target`, `gain` and `bpm` as decimals; `gain` may
    /// carry a sign).
    ///
    /// # Errors
    /// [`RecordError`]: a value over [`MAX_RECORD_BYTES`], a `v` other than 1, a missing
    /// required key (`v`, `app`, `mode`, `gain`, `trim`, `rate`; `stat` and `target` with a
    /// gain), a key twice, a part without `=`, `stat` or `target` with `gain=none`, `bpm=none`
    /// without `bar1=none` or the other way round, or a value
    /// that does not read (`mode` other than `prepare`/`library`, `stat` other than
    /// `S-P95`/`I`, a number that does not parse or is not finite, `src` not 16 hex digits, an
    /// empty `app`).
    pub fn parse(value: &str) -> Result<Self, RecordError> {
        let value = value.trim();
        if value.len() > MAX_RECORD_BYTES {
            return Err(RecordError::TooLong(value.len()));
        }
        let pairs = Pairs::split(value)?;
        let version = pairs.get("v").ok_or(RecordError::Missing("v"))?;
        if version.parse::<u32>().ok() != Some(SOUNDCHECK_RECORD_VERSION) {
            return Err(RecordError::UnsupportedVersion(version.to_owned()));
        }
        let app = pairs.required("app")?;
        if app.is_empty() {
            return Err(invalid("app", app));
        }
        let mode = match pairs.required("mode")? {
            "prepare" => BatchMode::Prepare,
            "library" => BatchMode::Library,
            other => return Err(invalid("mode", other)),
        };
        let gain = match pairs.required("gain")? {
            "none" => {
                for key in ["stat", "target"] {
                    if let Some(v) = pairs.get(key) {
                        return Err(invalid(key, v));
                    }
                }
                None
            }
            gain => Some(RecordGain {
                stat: match pairs.required("stat")? {
                    "S-P95" => LoudnessMode::Dj,
                    "I" => LoudnessMode::Streaming,
                    other => return Err(invalid("stat", other)),
                },
                target: Lufs(decimal("target", pairs.required("target")?)?),
                gain_db: decimal("gain", gain)?,
            }),
        };
        let grid_withheld = withheld(&pairs)?;
        Ok(Self {
            app: app.to_owned(),
            mode,
            gain,
            trim_frames: integer("trim", pairs.required("trim")?)?,
            sample_rate: integer("rate", pairs.required("rate")?)?,
            bpm: pairs
                .get("bpm")
                .filter(|_| !grid_withheld)
                .map(|v| decimal("bpm", v).map(Bpm))
                .transpose()?,
            bar1: pairs
                .get("bar1")
                .filter(|_| !grid_withheld)
                .map(|v| integer("bar1", v).map(SampleIndex))
                .transpose()?,
            grid_withheld,
            source_hash: pairs.get("src").map(hex8).transpose()?,
        })
    }
}

/// Whether the record withholds the grid: `bpm=none` and `bar1=none` together.
fn withheld(pairs: &Pairs<'_>) -> Result<bool, RecordError> {
    let (bpm, bar1) = (pairs.get("bpm"), pairs.get("bar1"));
    match (bpm == Some("none"), bar1 == Some("none")) {
        (true, true) => Ok(true),
        (false, false) => Ok(false),
        (true, false) => Err(bar1.map_or(RecordError::Missing("bar1"), |v| invalid("bar1", v))),
        (false, true) => Err(bpm.map_or(RecordError::Missing("bpm"), |v| invalid("bpm", v))),
    }
}

/// The record's name of a statistic.
fn stat_str(stat: LoudnessMode) -> &'static str {
    match stat {
        LoudnessMode::Dj => "S-P95",
        LoudnessMode::Streaming => "I",
    }
}

/// The record's keys, in the order they are written.
const KEYS: [&str; 11] = [
    "v", "app", "mode", "stat", "target", "gain", "trim", "rate", "bpm", "bar1", "src",
];

/// The known keys of a value and their values; unknown keys are dropped.
struct Pairs<'a>([Option<&'a str>; KEYS.len()]);

impl<'a> Pairs<'a> {
    fn split(value: &'a str) -> Result<Self, RecordError> {
        let mut found = [None; KEYS.len()];
        for part in value.split(';') {
            let (key, v) = part
                .split_once('=')
                .ok_or_else(|| RecordError::NotAPair(part.to_owned()))?;
            if let Some(i) = KEYS.iter().position(|k| *k == key) {
                if found[i].is_some() {
                    return Err(RecordError::Duplicate(key.to_owned()));
                }
                found[i] = Some(v);
            }
        }
        Ok(Self(found))
    }

    fn get(&self, key: &str) -> Option<&'a str> {
        KEYS.iter().position(|k| *k == key).and_then(|i| self.0[i])
    }

    fn required(&self, key: &'static str) -> Result<&'a str, RecordError> {
        self.get(key).ok_or(RecordError::Missing(key))
    }
}

fn invalid(key: &'static str, value: &str) -> RecordError {
    RecordError::Invalid {
        key,
        value: value.to_owned(),
    }
}

fn decimal(key: &'static str, value: &str) -> Result<f64, RecordError> {
    value
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| invalid(key, value))
}

fn integer<T: std::str::FromStr>(key: &'static str, value: &str) -> Result<T, RecordError> {
    // `FromStr` for integers accepts a leading `+`, which the record never writes.
    if value.starts_with('+') {
        return Err(invalid(key, value));
    }
    value.parse().map_err(|_| invalid(key, value))
}

fn hex8(value: &str) -> Result<[u8; 8], RecordError> {
    let bad = || invalid("src", value);
    if value.len() != 16 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let mut out = [0; 8];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
