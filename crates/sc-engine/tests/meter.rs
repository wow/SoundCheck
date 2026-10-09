//! The player's meter against the standards and the analysis: momentary loudness at every block
//! end equals an independent ITU-R BS.1770-5 computation (K-weighting from the analog prototype,
//! f64 mean square over the last 400 ms) within 0.005 LU, a 1 kHz sine at -20 dBFS reads
//! -20.0 LUFS, the true peak of each block is within 0.02 dB of an independent 4x-oversampled
//! peak over the same frames, and the largest block peak over a whole signal equals the analysis's
//! true peak within 0.005 dB. (The readings are rounded to 0.001 dB, so 0.0005 of each margin is
//! rounding.)
//!
//! EBU Tech 3341 cases 1 to 5 run the same comparisons on the EBU loudness test set, which is not
//! committed: fetch it with `scripts/fetch-ebu-testset.sh`, then
//! `SC_EBU_TESTSET=1 cargo test -p sc-engine --test meter -- --include-ignored`. Real music: with
//! `SC_REAL_FIXTURES=<dir>` (WAV, AIFF, FLAC and MP3 files; the folder is only read),
//! `cargo test -p sc-engine --release --test meter real -- --include-ignored --nocapture` runs the
//! momentary and track-peak comparisons on every file, one line per file.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sc_core::ipc::MeterFrame;
use sc_core::{AudioBuffer, AudioSpec, SampleIndex, testsig};
use sc_engine::player::{BLOCK_FRAMES, Metering, Meters};

/// Streams `buf` through the meter in player blocks; returns the frame after every block.
fn stream(buf: &AudioBuffer) -> Vec<MeterFrame> {
    let channels = usize::from(buf.spec.channels);
    let meters = Arc::new(Meters::new());
    let mut metering =
        Metering::new(buf.spec.sample_rate, buf.spec.channels, Arc::clone(&meters)).unwrap();
    let mut end = 0;
    buf.data
        .chunks(BLOCK_FRAMES * channels)
        .map(|block| {
            end += (block.len() / channels) as u64;
            metering.push(block, SampleIndex(end));
            meters.latest().unwrap()
        })
        .collect()
}

/// BS.1770-5 K-weighting, both stages, from the analog prototype (as libebur128 derives it for
/// any rate), applied to one channel in f64.
fn k_weighted(x: impl Iterator<Item = f32>, rate: u32) -> Vec<f64> {
    let fs = f64::from(rate);
    let shelf = {
        let (f0, g, q) = (
            1_681.974_450_955_533,
            3.999_843_853_973_347,
            0.707_175_236_955_419_6,
        );
        let k = (PI * f0 / fs).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        ]
    };
    let highpass = {
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (PI * f0 / fs).tan();
        let a0 = 1.0 + k / q + k * k;
        [
            1.0,
            -2.0,
            1.0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / q + k * k) / a0,
        ]
    };
    let biquad = |c: [f64; 5], input: Vec<f64>| {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        input
            .into_iter()
            .map(|x| {
                let y = c[0] * x + c[1] * x1 + c[2] * x2 - c[3] * y1 - c[4] * y2;
                (x2, x1, y2, y1) = (x1, x, y1, y);
                y
            })
            .collect::<Vec<f64>>()
    };
    biquad(highpass, biquad(shelf, x.map(f64::from).collect()))
}

/// Momentary loudness (LUFS) of the 400 ms ending at each of `ends`; mono counts twice (dual
/// mono). `None` for silence (at or below -70 LUFS) or before 400 ms.
fn reference_momentary(buf: &AudioBuffer, ends: &[u64]) -> Vec<Option<f64>> {
    let window = (u64::from(buf.spec.sample_rate) + 5) / 10 * 4;
    let channels = usize::from(buf.spec.channels);
    let weight = if channels == 1 { 2.0 } else { 1.0 };
    // Running sums of squares per channel, so each window is one subtraction.
    let sums: Vec<Vec<f64>> = (0..channels)
        .map(|c| {
            let y = k_weighted(buf.channel(c), buf.spec.sample_rate);
            let mut acc = vec![0.0];
            for v in y {
                acc.push(acc[acc.len() - 1] + v * v);
            }
            acc
        })
        .collect();
    ends.iter()
        .map(|&end| {
            if end < window {
                return None;
            }
            let (e, s) = (
                usize::try_from(end).unwrap(),
                usize::try_from(end - window).unwrap(),
            );
            #[allow(clippy::cast_precision_loss)]
            let energy: f64 = sums
                .iter()
                .map(|acc| (acc[e] - acc[s]) / window as f64)
                .sum();
            let energy = energy * weight;
            // At or below the absolute gate (-70 LUFS) is silence.
            Some(-0.691 + 10.0 * energy.log10()).filter(|&l| l > -70.0)
        })
        .collect()
}

/// Peak of `buf` 4x oversampled (Blackman-windowed sinc, 64 taps per phase), linear, per block
/// of [`BLOCK_FRAMES`] frames: the largest sample or interpolated point from each frame up to the
/// next. The meter's interpolator (48 taps, 4 phases) answers 6 frames after its input, so a
/// block's reading covers the points from 6 frames before the block to 6 frames before its end;
/// the reference covers the same.
fn reference_peaks(buf: &AudioBuffer) -> Vec<f64> {
    const HALF: i64 = 32;
    const INTERPOLATOR_DELAY: usize = 6;
    const FACTOR: i64 = 4;
    let taps: Vec<f64> = (-HALF * FACTOR..=HALF * FACTOR)
        .map(|k| {
            #[allow(clippy::cast_precision_loss)]
            let t = k as f64 / FACTOR as f64;
            #[allow(clippy::cast_precision_loss)]
            let w = 2.0 * PI * (k + HALF * FACTOR) as f64 / (2 * HALF * FACTOR) as f64;
            let window = 0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos();
            let sinc = if k == 0 {
                1.0
            } else {
                (PI * t).sin() / (PI * t)
            };
            sinc * window
        })
        .collect();
    let frames = buf.frames();
    let mut peaks = vec![0.0_f64; frames.div_ceil(BLOCK_FRAMES)];
    for c in 0..usize::from(buf.spec.channels) {
        let x: Vec<f64> = buf.channel(c).map(f64::from).collect();
        for n in 0..frames {
            let mut peak = x[n].abs();
            for p in 1..FACTOR {
                let mut y = 0.0;
                for m in -HALF + 1..=HALF {
                    let i = i64::try_from(n).unwrap() + m;
                    if let Ok(i) = usize::try_from(i)
                        && i < frames
                    {
                        let tap = usize::try_from((-m) * FACTOR + p + HALF * FACTOR).unwrap();
                        y += x[i] * taps[tap];
                    }
                }
                peak = peak.max(y.abs());
            }
            // The last frames' points come after the last block's reading.
            if let Some(block) = peaks.get_mut((n + INTERPOLATOR_DELAY) / BLOCK_FRAMES) {
                *block = block.max(peak);
            }
        }
    }
    peaks
}

fn ends(frames: &[MeterFrame]) -> Vec<u64> {
    frames.iter().map(|f| f.position.0).collect()
}

/// Every block end's momentary loudness within `tolerance` LU of the reference; the momentary
/// values are `None` exactly where the reference has none.
fn assert_momentary(buf: &AudioBuffer, frames: &[MeterFrame], tolerance: f64, what: &str) {
    let reference = reference_momentary(buf, &ends(frames));
    let mut compared = 0;
    let mut worst = 0.0_f64;
    for (f, r) in frames.iter().zip(&reference) {
        match (f.in_momentary, r) {
            (Some(m), Some(r)) => {
                assert!(
                    (m.0 - r).abs() <= tolerance,
                    "{what} at {}: {} vs {r:.4}",
                    f.position.0,
                    m.0
                );
                compared += 1;
                worst = worst.max((m.0 - r).abs());
            }
            (None, None) => {}
            (m, r) => panic!("{what} at {}: {m:?} vs {r:?}", f.position.0),
        }
    }
    assert!(compared > 0, "{what}: nothing compared");
    eprintln!("{what}: momentary within {worst:.5} LU of the reference at {compared} block ends");
}

/// Each block's true peak within 0.02 dB of the reference over the same frames.
fn assert_peaks(buf: &AudioBuffer, frames: &[MeterFrame], what: &str) {
    let mut worst = 0.0_f64;
    for (f, r) in frames.iter().zip(reference_peaks(buf)) {
        let reference = 20.0 * r.log10();
        let peak = f.in_peak.map_or(f64::NEG_INFINITY, |p| p.0);
        assert!(
            (peak - reference).abs() <= 0.02,
            "{what} at {}: {peak:.3} vs {reference:.3} dBTP",
            f.position.0
        );
        worst = worst.max((peak - reference).abs());
    }
    eprintln!("{what}: block true peaks within {worst:.4} dB of the reference");
}

/// The largest block peak equals the analysis's true peak within 0.005 dB.
fn assert_track_peak(buf: &AudioBuffer, frames: &[MeterFrame], what: &str) {
    let analysis = sc_analysis::loudness::measure(buf).unwrap().true_peak.0;
    let max = frames
        .iter()
        .filter_map(|f| f.in_peak)
        .fold(f64::NEG_INFINITY, |m, p| m.max(p.0));
    assert!(
        (max - analysis).abs() <= 0.005,
        "{what}: {max:.3} vs {analysis:.3}"
    );
    eprintln!("{what}: track peak {max:.3} vs analysis {analysis:.4} dBTP");
}

/// Sines at 220 Hz, 1 kHz, 3.1 kHz and 9.7 kHz whose levels move slowly, in stereo with the
/// channels at different levels: music-like, with peaks between samples in every block.
fn chord(spec: AudioSpec, seconds: f64) -> AudioBuffer {
    let frames = testsig::frames_for(spec, seconds);
    let rate = f64::from(spec.sample_rate);
    let mut data = Vec::with_capacity(frames * usize::from(spec.channels));
    for n in 0..frames {
        #[allow(clippy::cast_precision_loss)]
        let t = n as f64 / rate;
        let swell = 0.6 + 0.4 * (2.0 * PI * 0.7 * t).sin();
        let x = swell
            * (0.3 * (2.0 * PI * 220.0 * t).sin()
                + 0.2 * (2.0 * PI * 1_000.0 * t + 0.3).sin()
                + 0.15 * (2.0 * PI * 3_100.0 * t + 1.1).sin()
                + 0.1 * (2.0 * PI * 9_700.0 * t + 2.0).sin());
        for c in 0..spec.channels {
            #[allow(clippy::cast_possible_truncation)]
            data.push((x * if c == 0 { 1.0 } else { 0.7 }) as f32);
        }
    }
    AudioBuffer::new(spec, data)
}

#[test]
fn a_1_khz_sine_at_minus_20_dbfs_reads_minus_20_lufs() {
    let tone = testsig::sine(AudioSpec::CD, 1_000.0, 0.1, 2.0);
    let frames = stream(&tone);
    for f in &frames[20..frames.len() - 1] {
        let m = f.in_momentary.unwrap().0;
        assert!((m + 20.0).abs() <= 0.1, "{m}");
        assert!((f.in_peak.unwrap().0 + 20.0).abs() <= 0.05);
    }
    assert_momentary(&tone, &frames, 0.005, "sine");
}

#[test]
fn momentary_loudness_follows_bs_1770_at_every_block_end() {
    for spec in [
        AudioSpec::CD,
        AudioSpec::new(48_000, 2),
        AudioSpec::new(44_100, 1),
    ] {
        let music = chord(spec, 3.0);
        assert_momentary(&music, &stream(&music), 0.005, "chord");
    }
    // Noise bursts with silent gaps: the window fills, empties and fills again.
    let mut noise = testsig::seeded_noise(AudioSpec::CD, 7, 0.3, 3.0);
    for (i, frame) in noise.data.chunks_mut(2).enumerate() {
        if (i / 22_050) % 2 == 1 {
            frame.fill(0.0);
        }
    }
    assert_momentary(&noise, &stream(&noise), 0.005, "bursts");
}

#[test]
fn block_true_peaks_follow_a_4x_oversampled_reference() {
    for spec in [AudioSpec::CD, AudioSpec::new(48_000, 2)] {
        let music = chord(spec, 1.0);
        let frames = stream(&music);
        assert_peaks(&music, &frames, "chord");
        assert_track_peak(&music, &frames, "chord");
    }
    // A quarter-rate sine at 45 degrees (EBU Tech 3341 case 15): samples at 0.354, true peak
    // 0.5 (-6.02 dBTP).
    let quarter: Vec<f32> = (0..44_100)
        .flat_map(|n| {
            #[allow(clippy::cast_possible_truncation)]
            let x = (0.5 * (PI / 2.0 * f64::from(n) + PI / 4.0).sin()) as f32;
            [x, x]
        })
        .collect();
    let quarter = AudioBuffer::new(AudioSpec::CD, quarter);
    let frames = stream(&quarter);
    // EBU Tech 3341's true-peak tolerance: +0.2 / -0.4 dB.
    for f in &frames[1..] {
        let p = f.in_peak.unwrap().0 - 20.0 * 0.5_f64.log10();
        assert!((-0.4..=0.2).contains(&p), "{p:+.3} dB from -6.02 dBTP");
    }
    assert_track_peak(&quarter, &frames, "quarter-rate sine");
}

#[test]
fn two_runs_read_the_same() {
    let music = chord(AudioSpec::CD, 2.0);
    let first = stream(&music);
    assert_eq!(first, stream(&music), "deterministic");
}

const ENV: &str = "SC_EBU_TESTSET";

fn find(name: &str) -> PathBuf {
    fn walk(dir: &Path, name: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, name) {
                    return Some(found);
                }
            } else if path.file_name().and_then(|f| f.to_str()) == Some(name) {
                return Some(path);
            }
        }
        None
    }
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/ebu-loudness-test-set");
    walk(&dir, name).unwrap_or_else(|| panic!("{name} not under {}", dir.display()))
}

#[test]
#[ignore = "needs the EBU loudness test set: SC_EBU_TESTSET=1"]
fn tech_3341_cases_1_to_5_read_as_the_offline_meter() {
    if std::env::var_os(ENV).is_none() {
        eprintln!("skipped: set {ENV}=1 after fetching the EBU loudness test set");
        return;
    }
    for (file, steady) in [
        ("seq-3341-1-16bit.wav", Some(-23.0)),
        ("seq-3341-2-16bit.wav", Some(-33.0)),
        ("seq-3341-3-16bit-v02.wav", None),
        ("seq-3341-4-16bit-v02.wav", None),
        ("seq-3341-5-16bit-v02.wav", None),
    ] {
        let buf = sc_io::read_all(&find(file)).unwrap();
        let frames = stream(&buf);
        assert_momentary(&buf, &frames, 0.005, file);
        if let Some(level) = steady {
            for f in &frames[20..frames.len() - 1] {
                let m = f.in_momentary.unwrap().0;
                assert!((m - level).abs() <= 0.1, "{file}: {m} vs {level}");
            }
        }
        let report = sc_analysis::loudness::measure(&buf).unwrap();
        let loudest = frames
            .iter()
            .filter_map(|f| f.in_momentary)
            .fold(f64::NEG_INFINITY, |m, l| m.max(l.0));
        let offline = report.momentary_max.unwrap().0;
        assert!(
            (loudest - offline).abs() <= 0.05,
            "{file}: max {loudest} vs {offline}"
        );
        assert_track_peak(&buf, &frames, file);
    }
}

/// Every WAV, AIFF, FLAC or MP3 file directly in `dir`, skipping names starting with `.`.
fn audio_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(".");
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            p.is_file()
                && !name.starts_with('.')
                && matches!(
                    ext.as_deref(),
                    Some("wav" | "aif" | "aiff" | "flac" | "mp3")
                )
        })
        .collect();
    files.sort();
    files
}

#[test]
#[ignore = "needs a folder of real music: SC_REAL_FIXTURES=<dir>"]
fn real_music_reads_as_the_reference_and_the_analysis() {
    let Some(dir) = std::env::var_os("SC_REAL_FIXTURES") else {
        eprintln!("skipped: set SC_REAL_FIXTURES=<dir> to a folder of music");
        return;
    };
    let files = audio_files(Path::new(&dir));
    assert!(
        !files.is_empty(),
        "no audio files in {}",
        Path::new(&dir).display()
    );
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let buf = match sc_io::read_all(&path) {
            Ok(buf) => buf,
            Err(e) => {
                eprintln!("{name}: not read ({e})");
                continue;
            }
        };
        let frames = stream(&buf);
        assert_momentary(&buf, &frames, 0.005, &name);
        assert_track_peak(&buf, &frames, &name);
    }
}
