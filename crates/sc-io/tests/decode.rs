//! Decoding: a hound-written WAV, the committed FLAC/MP3/24-bit fixtures, streaming versus
//! whole-file equality, and every failure class the decoder reports.
//!
//! The fixtures under `tests/fixtures/audio/` are two-second lavfi sine tones rendered by ffmpeg
//! (`sine=frequency=1000:sample_rate=44100:duration=2`, stereo, and a one-second 440 Hz mono tone
//! at 48 kHz / 24-bit); the MP3 was encoded by libmp3lame at 128 kbit/s and carries a LAME tag.

use std::path::{Path, PathBuf};

use approx::assert_abs_diff_eq;
use sc_core::{AudioSpec, Error, testsig};
use sc_io::Decoder;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

fn write_wav(path: &Path, spec: hound::WavSpec, samples: &[f32]) {
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for s in samples {
        // Test fixture quantisation: the value is clamped so the cast cannot overflow.
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s).clamp(-1.0, 1.0) * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write sample");
    }
    w.finalize().expect("finalize wav");
}

fn wav_spec(channels: u16, sample_rate: u32) -> hound::WavSpec {
    hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

#[test]
fn decodes_a_sine_wav_with_the_right_spec_and_peak() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sine.wav");
    let original = testsig::sine(AudioSpec::CD, 440.0, 0.5, 0.25);
    write_wav(&path, wav_spec(2, 44_100), &original.data);

    let decoded = sc_io::read_all(&path).expect("decode");
    assert_eq!(decoded.spec, AudioSpec::CD);
    assert_eq!(decoded.frames(), original.frames());
    assert_abs_diff_eq!(decoded.peak_abs(), 0.5, epsilon = 1e-3);
}

#[test]
fn flac_fixture_decodes_to_exactly_two_seconds() {
    let flac = sc_io::read_all(&fixture("sine-2s.flac")).expect("decode flac");
    assert_eq!(flac.spec, AudioSpec::CD);
    assert_eq!(flac.frames(), 88_200);
    // lavfi's sine has amplitude 1/8; ffmpeg's mono-to-stereo upmix lowers it by 3 dB.
    assert_abs_diff_eq!(flac.peak_abs(), 0.125 / 2_f32.sqrt(), epsilon = 1e-3);
}

#[test]
fn mono_24_bit_wav_fixture_decodes_at_48k() {
    let wav = sc_io::read_all(&fixture("sine-1s-mono-24.wav")).expect("decode wav");
    assert_eq!(wav.spec, AudioSpec::new(48_000, 1));
    assert_eq!(wav.frames(), 48_000);
    // lavfi's sine has amplitude 1/8 and this fixture stays mono.
    assert_abs_diff_eq!(wav.peak_abs(), 0.125, epsilon = 1e-3);
}

/// The LAME tag says 576 encoder-delay frames; symphonia adds the 529-frame decoder delay and
/// discards both plus the padding, so the output is the encoder's input: exactly 2.000 s.
#[test]
fn mp3_fixture_is_gapless_after_lame_trim() {
    let decoder = Decoder::open(&fixture("lame-2s.mp3")).expect("open mp3");
    assert_eq!(decoder.spec(), AudioSpec::CD);
    assert_eq!(decoder.delay(), 576 + 529);
    assert!(decoder.padding() > 0, "padding {}", decoder.padding());
    assert_eq!(decoder.total_frames(), Some(88_200));

    let mp3 = sc_io::read_all(&fixture("lame-2s.mp3")).expect("decode mp3");
    assert_eq!(mp3.frames(), 88_200);
    // Same source and upmix as the FLAC fixture; 128 kbit/s keeps a pure tone within 1 %.
    let flac = sc_io::read_all(&fixture("sine-2s.flac")).expect("decode flac");
    assert_abs_diff_eq!(mp3.peak_abs(), flac.peak_abs(), epsilon = 0.01);
    // Nothing of the encoder's fade-in survives: the first block already holds the tone.
    let first_10_ms: f32 = mp3.data[..882].iter().map(|s| s.abs()).fold(0.0, f32::max);
    assert!(
        first_10_ms > 0.5 * mp3.peak_abs(),
        "first 10 ms peak {first_10_ms}"
    );
}

#[test]
fn streaming_blocks_concatenate_to_the_whole_file() {
    let path = fixture("sine-2s.flac");
    let whole = sc_io::read_all(&path).expect("read_all");
    let decoder = Decoder::open(&path).expect("open");
    let channels = usize::from(decoder.spec().channels);
    let mut streamed: Vec<f32> = Vec::new();
    let mut blocks = 0_usize;
    let delivered = decoder
        .for_each_block(|block| {
            assert!(!block.is_empty());
            assert_eq!(block.len() % channels, 0, "blocks are whole frames");
            streamed.extend_from_slice(block);
            blocks += 1;
            Ok(())
        })
        .expect("stream");
    assert!(
        blocks > 1,
        "a two-second FLAC decodes in more than one block"
    );
    assert_eq!(delivered, 88_200);
    assert_eq!(streamed, whole.data);
}

#[test]
fn six_channel_wav_is_rejected_as_unsupported_channels() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("six.wav");
    let samples = vec![0.1_f32; 6 * 4_410];
    write_wav(&path, wav_spec(6, 44_100), &samples);

    let err = sc_io::read_all(&path).unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedChannels { channels: 6, .. }),
        "{err}"
    );
}

#[test]
fn text_file_is_unsupported_format() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("notes.wav");
    std::fs::write(&path, "this is not audio\n".repeat(64)).expect("write");

    let err = sc_io::read_all(&path).unwrap_err();
    assert!(matches!(err, Error::UnsupportedFormat { .. }), "{err}");
}

#[test]
fn truncated_flac_is_corrupt() {
    let bytes = std::fs::read(fixture("sine-2s.flac")).expect("read fixture");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cut.flac");
    // Keep the whole header (ffmpeg writes an 8 KB padding block) and cut inside the frames.
    std::fs::write(&path, &bytes[..bytes.len() * 6 / 10]).expect("write");

    let err = sc_io::read_all(&path).unwrap_err();
    assert!(matches!(err, Error::Corrupt { .. }), "{err}");
    assert!(err.to_string().contains("truncated"), "{err}");
}

#[test]
fn missing_file_is_an_io_error() {
    let err = sc_io::read_all(Path::new("/nonexistent/file.wav")).unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err}");
}
