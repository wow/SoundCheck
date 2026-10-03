//! The apply harness: what a writer is asked to do, what it reports, the parameter grid every
//! writer runs, and the stand-ins for the `sc-io` writers until they exist.

use std::path::Path;

/// The five `bext` v2 loudness fields (EBU Tech 3285 v2), each `round(100 x value)` as a signed
/// 16-bit integer; a field that was not measured holds [`BextLoudness::UNMEASURED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BextLoudness {
    /// Integrated loudness, LUFS x 100.
    pub value: i16,
    /// Loudness range, LU x 100.
    pub range: i16,
    /// Maximum true peak, dBTP x 100.
    pub max_true_peak: i16,
    /// Maximum momentary loudness, LUFS x 100.
    pub max_momentary: i16,
    /// Maximum short-term loudness, LUFS x 100.
    pub max_short_term: i16,
}

impl BextLoudness {
    /// Tech 3285 v2: "If any of the loudness parameters are not being used then their 16-bit
    /// integer values shall be set to 7FFFh".
    pub const UNMEASURED: i16 = 0x7FFF;

    /// The ten little-endian bytes as stored at offset 412 of the chunk.
    #[must_use]
    pub fn bytes(&self) -> [u8; 10] {
        let mut out = [0_u8; 10];
        let fields = [
            self.value,
            self.range,
            self.max_true_peak,
            self.max_momentary,
            self.max_short_term,
        ];
        for (i, v) in fields.iter().enumerate() {
            out[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// Loudness a writer is given in the grid: -11.23 LUFS, range not measured, -0.52 dBTP,
/// -8.12 LUFS momentary, -9.05 LUFS short-term.
pub const LOUDNESS: BextLoudness = BextLoudness {
    value: -1123,
    range: BextLoudness::UNMEASURED,
    max_true_peak: -52,
    max_momentary: -812,
    max_short_term: -905,
};

/// What a single apply does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApplyArgs {
    /// Gain applied to every sample, dB.
    pub gain_db: f64,
    /// Frames removed from the start (0 = length preserved).
    pub trim_samples: u64,
    /// Output bits per sample; `None` keeps the source depth (24 for a float source).
    pub bits: Option<u8>,
    /// Loudness written into an existing `bext` chunk (never adds one).
    pub loudness: Option<BextLoudness>,
}

/// 0 dB, no trim, source depth, no loudness: an ideal writer's output equals the input's
/// audio and every other block.
pub const IDENTITY: ApplyArgs = ApplyArgs {
    gain_db: 0.0,
    trim_samples: 0,
    bits: None,
    loudness: None,
};

/// A tag item SoundCheck adds or replaces, by parser label (`TBPM`, `TXXX:BPM`, Vorbis
/// `REPLAYGAIN_TRACK_GAIN`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEdit {
    /// Frame label or Vorbis field name as `parse.rs` derives it.
    pub label: &'static str,
    /// Text value.
    pub value: String,
}

/// What a writer reports besides the output file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// Whether the requested tag edits were written (false when the tag cannot be edited
    /// safely, e.g. tag-level unsynchronisation; the tag is then carried unchanged).
    pub tags_added: bool,
}

/// The `SOUNDCHECK` record value: longer than 200 bytes, so its frame size differs between
/// the plain 32-bit (v2.3) and syncsafe (v2.4) encodings.
#[must_use]
pub fn soundcheck_record() -> String {
    let record = concat!(
        r#"{"schema":1,"version":"0.1.0","mode":"dj","statistic":"s-p95","#,
        r#""target_lufs":-11.0,"measured_lufs":-7.8,"gain_db":-3.2,"#,
        r#""ceiling_dbtp":-0.5,"true_peak_dbtp":-0.52,"trim_samples":441,"#,
        r#""bpm":128.0,"anchor_sample":441,"verdict":"static"}"#
    );
    assert!(record.len() >= 200, "the record must exceed 200 bytes");
    record.into()
}

/// ID3 edits: `TBPM` and existing `ReplayGain` frames replaced in place, the rest appended.
#[must_use]
pub fn id3_edits() -> Vec<TagEdit> {
    vec![
        TagEdit {
            label: "TBPM",
            value: "128".into(),
        },
        TagEdit {
            label: "TXXX:BPM",
            value: "128.00".into(),
        },
        TagEdit {
            label: "TXXX:REPLAYGAIN_TRACK_GAIN",
            value: "-6.20 dB".into(),
        },
        TagEdit {
            label: "TXXX:REPLAYGAIN_TRACK_PEAK",
            value: "0.912345".into(),
        },
        TagEdit {
            label: "TXXX:SOUNDCHECK",
            value: soundcheck_record(),
        },
    ]
}

/// Vorbis edits (field names compare case-insensitively with existing fields).
#[must_use]
pub fn vorbis_edits() -> Vec<TagEdit> {
    vec![
        TagEdit {
            label: "REPLAYGAIN_TRACK_GAIN",
            value: "-6.20 dB".into(),
        },
        TagEdit {
            label: "REPLAYGAIN_TRACK_PEAK",
            value: "0.912345".into(),
        },
        TagEdit {
            label: "SOUNDCHECK",
            value: soundcheck_record(),
        },
    ]
}

/// Writes `input` with `args` applied to `out`.
///
/// # Errors
/// The writer's refusal; no output file exists then.
pub fn apply(input: &Path, out: &Path, args: &ApplyArgs) -> Result<Applied, String> {
    apply_with_tags(input, out, args, &[])
}

/// Writes `input` with `args` applied and the tag items `edits` added or replaced.
///
/// # Errors
/// The writer's refusal; no output file exists then.
pub fn apply_with_tags(
    input: &Path,
    out: &Path,
    args: &ApplyArgs,
    edits: &[TagEdit],
) -> Result<Applied, String> {
    let _ = (out, args);
    if input.extension().is_some_and(|e| e == "flac") {
        unimplemented!("sc-io writer arrives with the FLAC writer");
    }
    if edits.is_empty() {
        unimplemented!("sc-io writer arrives with the IFF writer");
    }
    unimplemented!("sc-io tag editing arrives with the ID3 editor");
}

/// Whether the writer tests should run: they need `SC_FILE_WRITERS=1` in addition to
/// `--ignored`, so a nightly `--include-ignored` run stays green until the writers exist.
#[must_use]
pub fn writers_enabled(test: &str) -> bool {
    let on = std::env::var("SC_FILE_WRITERS").as_deref() == Ok("1");
    if !on {
        eprintln!("{test}: skipped; set SC_FILE_WRITERS=1 once the sc-io writers exist");
    }
    on
}

/// The gain x length x loudness rows every writer is run with.
#[must_use]
pub fn grid() -> Vec<ApplyArgs> {
    let mut v = Vec::new();
    for gain_db in [0.0, -3.2, 2.1] {
        for trim_samples in [0, 441] {
            for loudness in [None, Some(LOUDNESS)] {
                v.push(ApplyArgs {
                    gain_db,
                    trim_samples,
                    bits: None,
                    loudness,
                });
            }
        }
    }
    v
}
