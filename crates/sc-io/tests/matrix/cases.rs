//! The fixture matrix: every container layout the file layer must rewrite without loss, with
//! the expected fate of each block when SoundCheck applies gain, an optional head trim, `bext`
//! loudness and tag edits. The fixtures themselves are in `cases_iff.rs` and `cases_flac.rs`.
//!
//! Expectation rules (one place, applied to every fixture):
//! - Replaced: WAV `fmt ` and `data`, AIFF `COMM` and `SSND` (new header and audio; an `SSND`
//!   offset is normalised to 0), FLAC STREAMINFO, SEEKTABLE and the audio frames,
//!   `VORBIS_COMMENT` (re-emitted with every field carried, see the tag rules) and PADDING
//!   (resized to absorb new fields).
//! - Patched (same place, the named fields recomputed from the apply arguments, every other
//!   byte identical; see `expect.rs`): `cue ` and `smpl` positions, AIFF `MARK` positions and
//!   CUESHEET track offsets shift by the head trim (positions inside the cut clamp to 0, a
//!   loop ending inside the cut refuses the file); the lead-out and a PCM `fact` follow the new
//!   length; `bext` gets `TimeReference` plus the trim and, when loudness is given, Version 2 and
//!   its five loudness fields (an existing v0/v1 `bext` is upgraded, none is ever added).
//! - Dropped: RF64 `ds64` (the output is RIFF/WAVE), AIFF-C `FVER` (the output is FORM AIFF),
//!   `fact` of a float source (it describes non-PCM data; the output is integer PCM).
//! - Carried byte for byte, in order, with its pad byte value: everything else, including
//!   chunks never seen before, opaque DJ data (Serato GEOB and `SERATO_*`, iXML, `APPL`),
//!   ID3 extended headers, ID3 frames with any frame flags, Vorbis fields, an `ID3v2` tag in
//!   front of a FLAC stream and bytes after the container or stream (`ID3v1`).
//! - Tags (when edits are requested): only the first ID3 chunk of a file is edited (a second
//!   one is carried); an existing item with an edited label is replaced in place, the other
//!   edits are appended once after every carried item; a v2.3 extended header gets its
//!   padding-size field updated; a tag with tag-level unsynchronisation is not edited (carried
//!   unchanged, reported as "tags not added").
//! - A float source whose peak after gain reaches full scale is refused, never clipped.

use super::parse::{Container, Kind, Listed};
use super::pcm::Samples;
use super::riff::Built;

/// Frames per fixture: 0.45 s at 44.1 kHz; not a multiple of the FLAC block size.
pub const FRAMES: usize = 20_000;

/// What the writer must do with an input block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// Present in the output with identical bytes (and pad byte), in the same order.
    Carried,
    /// Present in the output at the same place with new content.
    Replaced,
    /// Present at the same place; only the named fields change, as computed from the apply
    /// arguments; every other byte is identical.
    Patched {
        /// Names of the fields that may change.
        fields: &'static [&'static str],
    },
    /// Absent from the output, for the stated reason.
    Dropped(&'static str),
}

impl Expect {
    /// Stable name used in the golden manifest.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Carried => "carried",
            Self::Replaced => "replaced",
            Self::Patched { .. } => "patched",
            Self::Dropped(_) => "dropped",
        }
    }
}

/// An input block and its expected fate.
#[derive(Debug, Clone)]
pub struct BlockExpect {
    /// The block as the builder wrote it.
    pub listed: Listed,
    /// Its fate on apply.
    pub expect: Expect,
}

/// Whether symphonia 0.6 decodes the fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Symphonia {
    /// It decodes to the generated samples.
    Decodes,
    /// It refuses the container (documented quirk; `sc-io` must read it itself).
    Refuses(&'static str),
    /// It decodes only the first `frames` frames (documented quirk; `sc-io` must work around
    /// it).
    Truncates {
        /// Frames symphonia delivers.
        frames: usize,
        /// Why.
        why: &'static str,
    },
}

/// One fixture of the matrix.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// Unique name, also the file stem in temp directories.
    pub name: &'static str,
    /// Container family.
    pub container: Container,
    /// File extension (probe hint).
    pub ext: &'static str,
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Sample frames.
    pub frames: usize,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Stored bits per sample (32 for float).
    pub bits: u8,
    /// The stored samples.
    pub source: Samples,
    /// Every block in file order with its fate.
    pub expected: Vec<BlockExpect>,
    /// Whether symphonia decodes it.
    pub symphonia: Symphonia,
}

impl Fixture {
    /// Whether the source is IEEE float.
    #[must_use]
    pub fn is_float(&self) -> bool {
        matches!(self.source, Samples::Float(_))
    }

    /// Whether the fixture is a WAV/RF64/AIFF/AIFF-C file.
    #[must_use]
    pub fn is_iff(&self) -> bool {
        self.container != Container::Flac
    }
}

const DS64: &str = "RF64 input is written as RIFF/WAVE, which has no ds64";
const FVER: &str = "AIFF-C version chunk; the output is FORM AIFF";
const FACT: &str = "fact describes non-PCM data; the output is integer PCM";

/// Fields a `cue ` patch may change.
pub const CUE_FIELDS: &[&str] = &["dwPosition", "dwSampleOffset"];
/// Fields an `smpl` patch may change.
pub const SMPL_FIELDS: &[&str] = &["dwStart", "dwEnd"];
/// Fields a `MARK` patch may change.
pub const MARK_FIELDS: &[&str] = &["position"];
/// Fields a PCM `fact` patch may change.
pub const FACT_FIELDS: &[&str] = &["dwSampleLength"];
/// Fields a `bext` patch may change.
pub const BEXT_FIELDS: &[&str] = &[
    "TimeReference",
    "Version",
    "LoudnessValue",
    "LoudnessRange",
    "MaxTruePeakLevel",
    "MaxMomentaryLoudness",
    "MaxShortTermLoudness",
];
/// Fields a CUESHEET patch may change.
pub const CUESHEET_FIELDS: &[&str] = &["track offset", "lead-out offset"];

fn rule(float: bool, l: &Listed) -> Expect {
    match (l.kind, l.id.as_str()) {
        (Kind::Chunk, "fmt " | "data" | "COMM" | "SSND")
        | (Kind::FlacBlock, "STREAMINFO" | "SEEKTABLE" | "VORBIS_COMMENT" | "PADDING")
        | (Kind::FlacFrames, _) => Expect::Replaced,
        (Kind::Chunk, "ds64") => Expect::Dropped(DS64),
        (Kind::Chunk, "FVER") => Expect::Dropped(FVER),
        (Kind::Chunk, "fact") if float => Expect::Dropped(FACT),
        (Kind::Chunk, "fact") => Expect::Patched {
            fields: FACT_FIELDS,
        },
        (Kind::Chunk, "cue ") => Expect::Patched { fields: CUE_FIELDS },
        (Kind::Chunk, "smpl") => Expect::Patched {
            fields: SMPL_FIELDS,
        },
        (Kind::Chunk, "MARK") => Expect::Patched {
            fields: MARK_FIELDS,
        },
        (Kind::Chunk, "bext") => Expect::Patched {
            fields: BEXT_FIELDS,
        },
        (Kind::FlacBlock, "CUESHEET") => Expect::Patched {
            fields: CUESHEET_FIELDS,
        },
        _ => Expect::Carried,
    }
}

/// What a fixture builder declares besides its bytes.
pub struct Spec {
    /// Fixture name.
    pub name: &'static str,
    /// Container family.
    pub container: Container,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Stored samples.
    pub source: Samples,
    /// Whether symphonia decodes it.
    pub symphonia: Symphonia,
}

/// A spec for a fixture symphonia decodes.
#[must_use]
pub fn spec(name: &'static str, container: Container, rate: u32, ch: u16, src: Samples) -> Spec {
    Spec {
        name,
        container,
        sample_rate: rate,
        channels: ch,
        source: src,
        symphonia: Symphonia::Decodes,
    }
}

/// Attaches the expectation rules to a built file.
#[must_use]
pub fn finish(spec: Spec, built: Built) -> Fixture {
    let float = matches!(spec.source, Samples::Float(_));
    let expected = built
        .listing
        .into_iter()
        .map(|listed| BlockExpect {
            expect: rule(float, &listed),
            listed,
        })
        .collect();
    let ext = match spec.container {
        Container::Wave | Container::Rf64 => "wav",
        Container::Aiff => "aiff",
        Container::Aifc => "aifc",
        Container::Flac => "flac",
    };
    Fixture {
        name: spec.name,
        container: spec.container,
        ext,
        bytes: built.bytes,
        frames: spec.source.len() / usize::from(spec.channels),
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits: spec.source.bits(),
        source: spec.source,
        expected,
        symphonia: spec.symphonia,
    }
}

/// The whole matrix, in a fixed order.
#[must_use]
pub fn matrix() -> Vec<Fixture> {
    let mut v = super::cases_iff::wav_fixtures();
    v.extend(super::cases_iff::aiff_fixtures());
    v.extend(super::cases_flac::flac());
    v
}

/// The fixture called `name`.
///
/// # Panics
/// When there is none.
#[must_use]
pub fn fixture(name: &str) -> Fixture {
    matrix()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
}
