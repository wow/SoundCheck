//! Decoding whole files into interleaved `f32`.

use std::fs::File;
use std::path::Path;

use sc_core::{AudioBuffer, AudioSpec, Error, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decodes the first audio track of `path` completely.
///
/// Encoder delay and padding for MP3/AAC are not yet applied; that lands with the analysis
/// milestone, and sample positions from this function must not be used for grids until then.
///
/// # Errors
/// [`Error::Io`] when the file cannot be opened or read, [`Error::UnsupportedFormat`] when no
/// decoder exists for it, [`Error::Corrupt`] when the container or stream is malformed.
pub fn read_all(path: &Path) -> Result<AudioBuffer> {
    let file = File::open(path).map_err(|source| Error::Io {
        path: path.into(),
        source,
    })?;
    let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| map_error(path, e))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| Error::UnsupportedFormat {
            path: path.into(),
            detail: "no audio track".into(),
        })?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| map_error(path, e))?;
    tracing::debug!(file = %path.display(), codec = ?track.codec_params.codec, "decoding");

    let mut spec: Option<AudioSpec> = None;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut data: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(map_error(path, e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let layout = *audio.spec();
                if spec.is_none() {
                    let channels = u16::try_from(layout.channels.count()).map_err(|_| {
                        Error::UnsupportedFormat {
                            path: path.into(),
                            detail: "too many channels".into(),
                        }
                    })?;
                    spec = Some(AudioSpec::new(layout.rate, channels));
                }
                let interleaved = sample_buf.get_or_insert_with(|| {
                    SampleBuffer::<f32>::new(audio.capacity() as u64, layout)
                });
                interleaved.copy_interleaved_ref(audio);
                data.extend_from_slice(interleaved.samples());
            }
            // A damaged packet is skipped, as symphonia recommends; the decoder recovers.
            Err(SymphoniaError::DecodeError(detail)) => {
                tracing::warn!(file = %path.display(), detail, "skipping undecodable packet");
            }
            Err(e) => return Err(map_error(path, e)),
        }
    }
    let spec = spec.ok_or_else(|| Error::Corrupt {
        path: path.into(),
        detail: "no decodable audio".into(),
    })?;
    Ok(AudioBuffer::new(spec, data))
}

fn map_error(path: &Path, err: SymphoniaError) -> Error {
    match err {
        SymphoniaError::IoError(source) => Error::Io {
            path: path.into(),
            source,
        },
        SymphoniaError::Unsupported(what) => Error::UnsupportedFormat {
            path: path.into(),
            detail: what.to_string(),
        },
        other => Error::Corrupt {
            path: path.into(),
            detail: other.to_string(),
        },
    }
}
