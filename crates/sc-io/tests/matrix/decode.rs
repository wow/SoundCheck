//! Decoding fixtures and outputs with symphonia directly (not through `sc-io`), into full-scale
//! `i32` (symphonia scales every integer format to the 32-bit range, so a `b`-bit sample `x`
//! reads as `x << (32 - b)`) and into `f32`.

use std::io::Cursor;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// A decoded stream.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Interleaved samples scaled to the full `i32` range.
    pub full_scale: Vec<i32>,
    /// Interleaved samples as `f32` (-1.0..1.0).
    pub float: Vec<f32>,
}

impl Decoded {
    /// Decoded frames.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.full_scale.len() / usize::from(self.channels.max(1))
    }

    /// The samples as `bits`-bit integers (the full-scale values shifted back down).
    #[must_use]
    pub fn as_bits(&self, bits: u8) -> Vec<i32> {
        let shift = 32 - u32::from(bits);
        self.full_scale.iter().map(|s| s >> shift).collect()
    }
}

/// Decodes the first audio track of an in-memory file; `ext` is the probe hint.
///
/// # Errors
/// symphonia's error, as text, when it refuses or fails on the file.
pub fn decode(bytes: &[u8], ext: &str) -> Result<Decoded, String> {
    let source = Cursor::new(bytes.to_vec());
    let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err("no audio codec parameters".into());
    };
    let track_id = track.id;
    let mut codec = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;
    let mut out = Decoded {
        sample_rate: 0,
        channels: 0,
        full_scale: Vec::new(),
        float: Vec::new(),
    };
    let (mut ints, mut floats) = (Vec::new(), Vec::new());
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(format!("packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = codec.decode(&packet).map_err(|e| format!("decode: {e}"))?;
        out.sample_rate = audio.spec().rate();
        out.channels = u16::try_from(audio.spec().channels().count()).map_err(|e| e.to_string())?;
        audio.copy_to_vec_interleaved::<i32>(&mut ints);
        audio.copy_to_vec_interleaved::<f32>(&mut floats);
        out.full_scale.extend_from_slice(&ints);
        out.float.extend_from_slice(&floats);
    }
    Ok(out)
}
