//! The writer tests and the runner they share with the self-tests.
//!
//! The IFF writer, ID3 edit and FLAC writer tests run in every `cargo test`. The same [`run`]
//! drives the reference writer in `selftest.rs`, so every assertion here is already exercised
//! on a correct output. `flac -t` (the reference decoder) checks the FLAC outputs when
//! `SC_FLAC_TOOLS=1` and the `flac` tool is installed.

use std::path::Path;

use super::apply::{
    self, Applied, ApplyArgs, IDENTITY, Refusal, TagEdit, apply, apply_with_tags, grid,
};
use super::cases::{Fixture, matrix};
use super::check::check_output;
use super::expect::refusal;
use super::golden::{OutputEntry, row_label};
use super::parse;

/// What a writer returns: output bytes and report, or its refusal.
pub type Written = Result<(Vec<u8>, Applied), Refusal>;

/// A writer under test.
pub type Writer<'a> = dyn Fn(&Fixture, &ApplyArgs, &[TagEdit]) -> Written + 'a;

/// Runs `writer` twice on `fx` and checks it: refused exactly when [`refusal`] says so and
/// with the error class it names, otherwise identical bytes and report on both runs and every check of `check_output`.
///
/// # Errors
/// What went wrong, with the fixture name and row.
pub fn run(
    writer: &Writer,
    fx: &Fixture,
    args: &ApplyArgs,
    edits: &[TagEdit],
) -> Result<(), String> {
    let first = writer(fx, args, edits);
    let second = writer(fx, args, edits);
    match (refusal(fx, args), first, second) {
        (Some(want), Err(a), Err(b)) if a.kind == want.kind && b.kind == want.kind => Ok(()),
        (Some(want), Err(got), _) | (Some(want), _, Err(got)) => Err(format!(
            "{} {args:?}: refused as {} ({}), want {} ({})",
            fx.name, got.kind, got.reason, want.kind, want.reason
        )),
        (Some(want), _, _) => Err(format!(
            "{} {args:?}: must be refused as {} ({})",
            fx.name, want.kind, want.reason
        )),
        (None, Ok((a, report_a)), Ok((b, report_b))) => {
            if a != b || report_a != report_b {
                return Err(format!("{} {args:?}: two runs differ", fx.name));
            }
            check_output(fx, &a, args, edits, report_a)
        }
        (None, Err(e), _) | (None, _, Err(e)) => Err(format!(
            "{} {args:?}: refused unexpectedly as {}: {}",
            fx.name, e.kind, e.reason
        )),
    }
}

/// Rows for the bit-depth test: 16 and 24 bits at 0 dB, and 16 bits with gain.
#[must_use]
pub fn bit_depth_rows() -> Vec<ApplyArgs> {
    vec![
        ApplyArgs {
            bits: Some(16),
            ..IDENTITY
        },
        ApplyArgs {
            bits: Some(24),
            ..IDENTITY
        },
        ApplyArgs {
            bits: Some(16),
            gain_db: -3.2,
            ..IDENTITY
        },
    ]
}

/// Rows for the tag tests: identity, and gain with trim and loudness.
#[must_use]
pub fn tag_rows() -> Vec<ApplyArgs> {
    vec![
        IDENTITY,
        ApplyArgs {
            gain_db: -3.2,
            trim_samples: 441,
            bits: None,
            loudness: Some(apply::LOUDNESS),
        },
    ]
}

/// Fixtures that hold at least one ID3 chunk.
#[must_use]
pub fn has_id3(fx: &Fixture) -> bool {
    parse::parse(&fx.bytes).is_ok_and(|p| !p.tags.is_empty())
}

/// The `sc-io` writer through the harness: writes the fixture into `dir`, applies, reads the
/// output back; a refusal must leave no output file.
fn sc_io(dir: &Path) -> impl Fn(&Fixture, &ApplyArgs, &[TagEdit]) -> Written {
    move |fx, args, edits| {
        let input = dir.join(format!("{}.{}", fx.name, fx.ext));
        std::fs::write(&input, &fx.bytes).expect("write the fixture");
        let out = dir.join(format!("{}-out.{}", fx.name, fx.ext));
        let _ = std::fs::remove_file(&out);
        let result = if edits.is_empty() {
            apply(&input, &out, args)
        } else {
            apply_with_tags(&input, &out, args, edits)
        };
        match result {
            Ok(applied) => Ok((std::fs::read(&out).expect("read the output"), applied)),
            Err(e) => {
                assert!(!out.exists(), "{}: a refusal left an output file", fx.name);
                Err(e)
            }
        }
    }
}

/// Runs the `sc-io` writer on every fixture and row.
fn run_rows(fixtures: &[Fixture], rows: &[ApplyArgs], edits: &[TagEdit]) {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = sc_io(dir.path());
    for fx in fixtures {
        for args in rows {
            run(&writer, fx, args, edits).unwrap_or_else(|e| panic!("{e}"));
        }
    }
}

/// SHA-256 of the `sc-io` output of every IFF fixture for every grid and bit-depth row, in
/// matrix order (`refused:<error kind>` where the writer refuses), for the golden manifest.
#[must_use]
pub fn iff_outputs(fixtures: &[Fixture]) -> Vec<OutputEntry> {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = sc_io(dir.path());
    let rows: Vec<ApplyArgs> = grid().into_iter().chain(bit_depth_rows()).collect();
    let mut out = Vec::new();
    for fx in fixtures.iter().filter(|f| f.is_iff()) {
        for args in &rows {
            let sha256 = match writer(fx, args, &[]) {
                Ok((bytes, _)) => parse::sha256_hex(&bytes),
                Err(refusal) => format!("refused:{}", refusal.kind),
            };
            out.push(OutputEntry {
                fixture: fx.name.into(),
                row: row_label(args),
                sha256,
            });
        }
    }
    out
}

/// SHA-256 of the `sc-io` output of every fixture holding an ID3 chunk for every tag row with
/// the ID3 edits, in matrix order, for the golden manifest (after [`iff_outputs`]).
#[must_use]
pub fn id3_outputs(fixtures: &[Fixture]) -> Vec<OutputEntry> {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = sc_io(dir.path());
    let edits = apply::id3_edits();
    let mut out = Vec::new();
    for fx in fixtures.iter().filter(|f| f.is_iff() && has_id3(f)) {
        for args in &tag_rows() {
            let sha256 = match writer(fx, args, &edits) {
                Ok((bytes, _)) => parse::sha256_hex(&bytes),
                Err(refusal) => format!("refused:{}", refusal.kind),
            };
            out.push(OutputEntry {
                fixture: fx.name.into(),
                row: format!("{}, id3 edits", row_label(args)),
                sha256,
            });
        }
    }
    out
}

#[test]
fn iff_apply_carries_every_chunk_and_writes_dj_safe_headers() {
    let fixtures: Vec<_> = matrix().into_iter().filter(Fixture::is_iff).collect();
    run_rows(&fixtures, &grid(), &[]);
}

#[test]
fn iff_apply_writes_the_requested_bit_depth() {
    let fixtures: Vec<_> = matrix().into_iter().filter(Fixture::is_iff).collect();
    run_rows(&fixtures, &bit_depth_rows(), &[]);
}

/// Every IFF fixture with the ID3 edits: a tagged file gets our frames (or, where the matrix
/// says the tag cannot be edited, keeps it unchanged), an untagged one renders as without
/// edits and reports the tags as not added.
#[test]
fn id3_edit_adds_our_frames_and_keeps_every_other_frame() {
    let fixtures: Vec<_> = matrix().into_iter().filter(Fixture::is_iff).collect();
    assert!(fixtures.iter().filter(|f| has_id3(f)).count() >= 8);
    run_rows(&fixtures, &tag_rows(), &apply::id3_edits());
}

/// Every FLAC fixture over the grid with the Vorbis edits.
#[test]
fn flac_apply_carries_blocks_and_rebuilds_streaminfo() {
    let fixtures: Vec<_> = matrix().into_iter().filter(|f| !f.is_iff()).collect();
    assert_eq!(fixtures.len(), 4);
    run_rows(&fixtures, &grid(), &apply::vorbis_edits());
}

#[test]
fn flac_apply_writes_the_requested_bit_depth() {
    let fixtures: Vec<_> = matrix().into_iter().filter(|f| !f.is_iff()).collect();
    run_rows(&fixtures, &bit_depth_rows(), &[]);
}

/// SHA-256 of the `sc-io` output of every FLAC fixture for every grid and bit-depth row, then
/// for every tag row with the Vorbis edits, in matrix order, for the golden manifest (after
/// [`id3_outputs`]).
#[must_use]
pub fn flac_outputs(fixtures: &[Fixture]) -> Vec<OutputEntry> {
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = sc_io(dir.path());
    let plain: Vec<ApplyArgs> = grid().into_iter().chain(bit_depth_rows()).collect();
    let edits = apply::vorbis_edits();
    let mut out = Vec::new();
    for fx in fixtures.iter().filter(|f| !f.is_iff()) {
        let rows = plain
            .iter()
            .map(|a| (a, &[][..], String::new()))
            .chain(
                tag_rows()
                    .iter()
                    .map(|a| (a, &edits[..], ", vorbis edits".into())),
            )
            .map(|(a, e, suffix)| (*a, e.to_vec(), suffix))
            .collect::<Vec<_>>();
        for (args, edits, suffix) in rows {
            let sha256 = match writer(fx, &args, &edits) {
                Ok((bytes, _)) => parse::sha256_hex(&bytes),
                Err(refusal) => format!("refused:{}", refusal.kind),
            };
            out.push(OutputEntry {
                fixture: fx.name.into(),
                row: format!("{}{suffix}", row_label(&args)),
                sha256,
            });
        }
    }
    out
}

/// `flac -t` (libFLAC's own decoder, MD5 included) accepts every FLAC output whose input had no
/// tags around the stream (libFLAC warns about an `ID3v2` tag and reports lost sync at an
/// `ID3v1` tag even on the untouched input). Opt-in: `SC_FLAC_TOOLS=1` and `flac` installed.
#[test]
fn flac_outputs_pass_flac_t() {
    if std::env::var("SC_FLAC_TOOLS").as_deref() != Ok("1") {
        eprintln!("flac_outputs_pass_flac_t: skipped; set SC_FLAC_TOOLS=1 to run `flac -t`");
        return;
    }
    if std::process::Command::new("flac")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("flac_outputs_pass_flac_t: skipped; the flac tool is not installed");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let writer = sc_io(dir.path());
    let rows: Vec<ApplyArgs> = grid().into_iter().chain(bit_depth_rows()).collect();
    let mut tested = 0;
    for fx in matrix().iter().filter(|f| !f.is_iff()) {
        let wrapped = parse::parse(&fx.bytes).is_ok_and(|p| {
            p.blocks
                .iter()
                .any(|b| matches!(b.kind, parse::Kind::LeadingTag | parse::Kind::Trailing))
        });
        if wrapped {
            continue;
        }
        for args in &rows {
            let (bytes, _) = writer(fx, args, &apply::vorbis_edits()).expect("renders");
            let path = dir.path().join(format!("{}-t.flac", fx.name));
            std::fs::write(&path, &bytes).expect("write");
            let run = std::process::Command::new("flac")
                .args(["-t", "-s", "-w"])
                .arg(&path)
                .output()
                .expect("run flac");
            assert!(
                run.status.success(),
                "{} {args:?}: flac -t failed: {}",
                fx.name,
                String::from_utf8_lossy(&run.stderr)
            );
            tested += 1;
        }
    }
    assert_eq!(tested, 3 * rows.len());
}
