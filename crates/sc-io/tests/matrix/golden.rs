//! The golden manifest of the matrix: per fixture its size, total SHA-256 and per-block kind,
//! id, SHA-256, pad byte and expected fate. The builders are deterministic, so the comparison
//! is exact; `UPDATE_GOLDEN=1` rewrites the file instead of comparing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::apply::{ApplyArgs, grid};
use super::cases::{Expect, Fixture};
use super::expect::patched;
use super::parse::{self, sha256_hex};

/// Manifest schema version; bump when the layout of the JSON changes.
const SCHEMA: u32 = 3;

/// The whole manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Layout version of this JSON.
    pub schema: u32,
    /// One entry per fixture, in matrix order.
    pub fixtures: Vec<FixtureEntry>,
    /// SHA-256 of each writer output per fixture and grid row. Empty until the `sc-io` writers
    /// exist; then the writer tests fill it (with `UPDATE_GOLDEN=1`) so any byte change of an
    /// output shows up as a reviewed diff.
    #[serde(default)]
    pub outputs: Vec<OutputEntry>,
    /// SHA-256 of every patched block's expected payload per fixture and grid row, so a change
    /// of the patch rules shows up as a reviewed diff.
    pub patched: Vec<PatchedEntry>,
}

/// The expected payload of one patched block for one grid row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchedEntry {
    /// Fixture name.
    pub fixture: String,
    /// The apply arguments, as text.
    pub row: String,
    /// Block id.
    pub id: String,
    /// SHA-256 of the expected payload.
    pub sha256: String,
}

/// A grid row as text.
#[must_use]
pub fn row_label(args: &ApplyArgs) -> String {
    format!(
        "gain {:+.1} dB, trim {}, bits {}, loudness {}",
        args.gain_db,
        args.trim_samples,
        args.bits.map_or("source".into(), |b| b.to_string()),
        if args.loudness.is_some() {
            "given"
        } else {
            "none"
        }
    )
}

fn patched_entries(fixtures: &[Fixture]) -> Vec<PatchedEntry> {
    let mut out = Vec::new();
    for fx in fixtures {
        let input = parse::parse(&fx.bytes).expect("fixtures parse");
        for args in grid() {
            for (block, e) in input.blocks.iter().zip(&fx.expected) {
                if let Expect::Patched { .. } = e.expect {
                    let payload = patched(&block.id, &block.bytes, fx, &args).expect("patch");
                    out.push(PatchedEntry {
                        fixture: fx.name.into(),
                        row: row_label(&args),
                        id: block.id.clone(),
                        sha256: sha256_hex(&payload),
                    });
                }
            }
        }
    }
    out
}

/// The output of one writer run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputEntry {
    /// Fixture name.
    pub fixture: String,
    /// The apply arguments and edits, as text.
    pub row: String,
    /// SHA-256 of the output file.
    pub sha256: String,
}

/// One fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureEntry {
    /// Fixture name.
    pub name: String,
    /// File size, bytes.
    pub bytes: usize,
    /// SHA-256 of the whole file.
    pub sha256: String,
    /// Sample frames.
    pub frames: usize,
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Stored bits per sample.
    pub bits: u8,
    /// Blocks in file order.
    pub blocks: Vec<BlockEntry>,
}

/// One block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEntry {
    /// Block kind (`chunk`, `id3-frame`, `flac-block`, ...).
    pub kind: String,
    /// Block id.
    pub id: String,
    /// SHA-256 of the hashed bytes.
    pub sha256: String,
    /// Value of the pad byte after an odd-length chunk, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<u8>,
    /// `carried`, `replaced` or `dropped`.
    pub expect: String,
    /// Fields a patched block may change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    /// Why a block is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The manifest of `fixtures`.
#[must_use]
pub fn manifest(fixtures: &[Fixture]) -> Manifest {
    Manifest {
        schema: SCHEMA,
        fixtures: fixtures
            .iter()
            .map(|f| FixtureEntry {
                name: f.name.into(),
                bytes: f.bytes.len(),
                sha256: sha256_hex(&f.bytes),
                frames: f.frames,
                sample_rate: f.sample_rate,
                channels: f.channels,
                bits: f.bits,
                blocks: f
                    .expected
                    .iter()
                    .map(|b| BlockEntry {
                        kind: b.listed.kind.name().into(),
                        id: b.listed.id.clone(),
                        sha256: b.listed.sha256.clone(),
                        pad: b.listed.pad,
                        expect: b.expect.name().into(),
                        fields: match b.expect {
                            Expect::Patched { fields } => {
                                Some(fields.iter().map(ToString::to_string).collect())
                            }
                            _ => None,
                        },
                        reason: match b.expect {
                            Expect::Dropped(why) => Some(why.into()),
                            _ => None,
                        },
                    })
                    .collect(),
            })
            .collect(),
        outputs: Vec::new(),
        patched: patched_entries(fixtures),
    }
}

/// `tests/fixtures/golden/file-layer-matrix.json` at the repository root.
#[must_use]
pub fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/golden/file-layer-matrix.json")
}

/// The first difference between two manifests, described for a failure message.
fn first_difference(want: &Manifest, got: &Manifest) -> String {
    if want.schema != got.schema {
        return format!("schema {} != {}", want.schema, got.schema);
    }
    for (w, g) in want.fixtures.iter().zip(&got.fixtures) {
        if w == g {
            continue;
        }
        for (i, (wb, gb)) in w.blocks.iter().zip(&g.blocks).enumerate() {
            if wb != gb {
                return format!("{}: block {i}: golden {wb:?}, built {gb:?}", w.name);
            }
        }
        return format!(
            "{}: golden {} bytes / {} blocks / {}, built {} bytes / {} blocks / {}",
            w.name,
            w.bytes,
            w.blocks.len(),
            w.sha256,
            g.bytes,
            g.blocks.len(),
            g.sha256
        );
    }
    for (w, g) in want.patched.iter().zip(&got.patched) {
        if w != g {
            return format!("patched payload differs: golden {w:?}, built {g:?}");
        }
    }
    format!(
        "golden has {} fixtures and {} patched payloads, built {} and {}",
        want.fixtures.len(),
        want.patched.len(),
        got.fixtures.len(),
        got.patched.len()
    )
}

/// Compares the built matrix with the golden file, or rewrites it with `UPDATE_GOLDEN=1`.
///
/// # Errors
/// The first difference, or why the file could not be read or written.
pub fn check_or_update(fixtures: &[Fixture]) -> Result<(), String> {
    let got = manifest(fixtures);
    let path = path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        let mut json = serde_json::to_string_pretty(&got).map_err(|e| e.to_string())?;
        json.push('\n');
        return std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e} (create it with UPDATE_GOLDEN=1)", path.display()))?;
    let want: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if want == got {
        Ok(())
    } else {
        Err(first_difference(&want, &got))
    }
}
