//! EBU Tech 3341 / Tech 3342 conformance on the EBU loudness test set (v05).
//!
//! The files are not committed. Run `scripts/fetch-ebu-testset.sh` (or unzip the set by hand into
//! `tests/fixtures/ebu-loudness-test-set/`), then
//! `SC_EBU_TESTSET=1 cargo test -p sc-analysis --test conformance_ebu -- --include-ignored`.
//! Tolerances are the standards' own: +/-0.1 LU for M, S and I; +/-1 LU for LRA; +0.2/-0.4 dB
//! for true peak. Five- and six-channel files are outside SoundCheck's scope and not run.

use std::path::{Path, PathBuf};

use sc_core::analysis::LoudnessReport;

const ENV: &str = "SC_EBU_TESTSET";

fn set_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ebu-loudness-test-set")
}

/// True when the set is requested; prints why the test does nothing otherwise.
fn requested() -> bool {
    if std::env::var_os(ENV).is_some() {
        true
    } else {
        eprintln!("skipped: set {ENV}=1 after fetching the EBU loudness test set");
        false
    }
}

/// Finds `name` anywhere under the set directory; the zip may unpack into a subfolder and a few
/// files carry a doubled `.wav.wav` suffix.
fn find(name: &str) -> PathBuf {
    fn walk(dir: &Path, name: &str, alt: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, name, alt) {
                    return Some(found);
                }
            } else if let Some(file) = path.file_name().and_then(|f| f.to_str())
                && (file == name || file == alt)
            {
                return Some(path);
            }
        }
        None
    }
    let alt = format!("{name}.wav");
    walk(&set_dir(), name, &alt).unwrap_or_else(|| {
        panic!(
            "{name} not found under {}; run scripts/fetch-ebu-testset.sh",
            set_dir().display()
        )
    })
}

fn report(name: &str) -> LoudnessReport {
    let buf = sc_io::read_all(&find(name)).expect("decode EBU file");
    sc_analysis::loudness::measure(&buf).expect("measure")
}

fn assert_lu(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual:.3} vs {expected:.1} (+/-{tolerance})"
    );
}

#[test]
#[ignore = "needs the EBU loudness test set: SC_EBU_TESTSET=1"]
fn tech_3341_cases_1_to_5_integrated_momentary_short_term() {
    if !requested() {
        return;
    }
    for (file, expected) in [
        ("seq-3341-1-16bit.wav", -23.0),
        ("seq-3341-2-16bit.wav", -33.0),
        ("seq-3341-3-16bit-v02.wav", -23.0),
        ("seq-3341-4-16bit-v02.wav", -23.0),
        ("seq-3341-5-16bit-v02.wav", -23.0),
    ] {
        let r = report(file);
        assert_lu(r.integrated.unwrap().0, expected, 0.1, &format!("{file} I"));
    }
    let r = report("seq-3341-1-16bit.wav");
    assert_lu(r.momentary_max.unwrap().0, -23.0, 0.1, "3341-1 max M");
    assert_lu(r.short_term_max.unwrap().0, -23.0, 0.1, "3341-1 max S");
    let r = report("seq-3341-2-16bit.wav");
    assert_lu(r.momentary_max.unwrap().0, -33.0, 0.1, "3341-2 max M");
    assert_lu(r.short_term_max.unwrap().0, -33.0, 0.1, "3341-2 max S");
}

#[test]
#[ignore = "needs the EBU loudness test set: SC_EBU_TESTSET=1"]
fn tech_3341_cases_7_8_9_12_programme_material() {
    if !requested() {
        return;
    }
    let r = report("seq-3341-7_seq-3342-5-24bit.wav");
    assert_lu(r.integrated.unwrap().0, -23.0, 0.1, "3341-7 I");
    let r = report("seq-3341-2011-8_seq-3342-6-24bit-v02.wav");
    assert_lu(r.integrated.unwrap().0, -23.0, 0.1, "3341-8 I");
    let r = report("seq-3341-9-24bit.wav");
    assert_lu(r.short_term_max.unwrap().0, -23.0, 0.1, "3341-9 max S");
    let r = report("seq-3341-12-24bit.wav");
    assert_lu(r.momentary_max.unwrap().0, -23.0, 0.1, "3341-12 max M");
}

#[test]
#[ignore = "needs the EBU loudness test set: SC_EBU_TESTSET=1"]
fn tech_3341_cases_10_and_13_window_maxima() {
    if !requested() {
        return;
    }
    for i in 1..=20 {
        let r = report(&format!("seq-3341-10-{i}-24bit.wav"));
        assert_lu(
            r.short_term_max.unwrap().0,
            -23.0,
            0.1,
            &format!("3341-10-{i} max S"),
        );
        let r = report(&format!("seq-3341-13-{i}-24bit.wav"));
        assert_lu(
            r.momentary_max.unwrap().0,
            -23.0,
            0.1,
            &format!("3341-13-{i} max M"),
        );
    }
}

#[test]
#[ignore = "needs the EBU loudness test set: SC_EBU_TESTSET=1"]
fn tech_3341_cases_15_to_23_true_peak() {
    if !requested() {
        return;
    }
    for (file, expected) in [
        ("seq-3341-15-24bit.wav", -6.0),
        ("seq-3341-16-24bit.wav", -6.0),
        ("seq-3341-17-24bit.wav", -6.0),
        ("seq-3341-18-24bit.wav", -6.0),
        ("seq-3341-19-24bit.wav", 3.0),
        ("seq-3341-20-24bit.wav", 0.0),
        ("seq-3341-21-24bit.wav", 0.0),
        ("seq-3341-22-24bit.wav", 0.0),
        ("seq-3341-23-24bit.wav", 0.0),
    ] {
        let tp = report(file).true_peak.0;
        assert!(
            (expected - 0.4..=expected + 0.2).contains(&tp),
            "{file} true peak {tp:.3} vs {expected:.1} (+0.2/-0.4)"
        );
    }
}

#[test]
#[ignore = "needs the EBU loudness test set: SC_EBU_TESTSET=1"]
fn tech_3342_loudness_range() {
    if !requested() {
        return;
    }
    for (file, expected) in [
        ("seq-3342-1-16bit.wav", 10.0),
        ("seq-3342-2-16bit.wav", 5.0),
        ("seq-3342-3-16bit.wav", 20.0),
        ("seq-3342-4-16bit.wav", 15.0),
        ("seq-3341-7_seq-3342-5-24bit.wav", 5.0),
        ("seq-3341-2011-8_seq-3342-6-24bit-v02.wav", 15.0),
    ] {
        let lra = report(file).lra.unwrap().0;
        assert_lu(lra, expected, 1.0, &format!("{file} LRA"));
    }
}
