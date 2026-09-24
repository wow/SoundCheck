//! A WAV written by hound decodes to the same samples.

use approx::assert_abs_diff_eq;
use sc_core::{AudioSpec, testsig};

fn write_wav(path: &std::path::Path, buf: &sc_core::AudioBuffer) {
    let spec = hound::WavSpec {
        channels: buf.spec.channels,
        sample_rate: buf.spec.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for s in &buf.data {
        // Test fixture quantisation: the value is clamped so the cast cannot overflow.
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s).clamp(-1.0, 1.0) * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write sample");
    }
    w.finalize().expect("finalize wav");
}

#[test]
fn decodes_a_sine_wav_with_the_right_spec_and_peak() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sine.wav");
    let original = testsig::sine(AudioSpec::CD, 440.0, 0.5, 0.25);
    write_wav(&path, &original);

    let decoded = sc_io::read_all(&path).expect("decode");
    assert_eq!(decoded.spec, AudioSpec::CD);
    assert_eq!(decoded.frames(), original.frames());
    assert_abs_diff_eq!(decoded.peak_abs(), 0.5, epsilon = 1e-3);
}

#[test]
fn missing_file_is_an_io_error() {
    let err = sc_io::read_all(std::path::Path::new("/nonexistent/file.wav")).unwrap_err();
    assert!(matches!(err, sc_core::Error::Io { .. }), "{err}");
}
