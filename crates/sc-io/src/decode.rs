//! Decoding through symphonia: blocks of interleaved `f32` with the encoder delay and padding
//! already removed, plus a whole-file convenience for tests and small tools.
//!
//! Gapless trimming (the LAME `delay`/`padding` pair of MP3, edit lists of MP4) is applied by
//! symphonia 0.6 itself: the demuxer marks on every packet how many frames to discard and the
//! decoder drops them, so the first sample this module delivers is the first sample the encoder
//! was given. The amounts are still reported by [`Decoder::delay`] and [`Decoder::padding`] for
//! the analysis record, and a test on a LAME-encoded fixture pins the behaviour.

use std::fs::File;
use std::path::{Path, PathBuf};

use sc_core::{AudioBuffer, AudioSpec, Error, Result};
use symphonia::core::audio::Channels;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// How many frames a stream may end short of its declared length before the file is reported as
/// corrupt; some encoders declare a length that is off by one block.
const TRUNCATION_TOLERANCE_FRAMES: u64 = 4096;

/// A streaming decoder over the first audio track of a file.
///
/// Blocks are delivered as they are decoded, so memory stays at one packet regardless of file
/// length. Only mono and stereo are accepted; the stream parameters are taken from the first
/// decoded packet, which [`Decoder::open`] already reads.
pub struct Decoder {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    codec: Box<dyn AudioDecoder>,
    track_id: u32,
    spec: AudioSpec,
    total_frames: Option<u64>,
    delay: u32,
    padding: u32,
    /// Interleaved samples of the first packet, decoded by `open` to learn the stream
    /// parameters; delivered before any further packet.
    pending: Vec<f32>,
    scratch: Vec<f32>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("path", &self.path)
            .field("spec", &self.spec)
            .field("total_frames", &self.total_frames)
            .field("delay", &self.delay)
            .field("padding", &self.padding)
            .finish_non_exhaustive()
    }
}

impl Decoder {
    /// Opens `path`, prepares the first audio track and decodes its first packet.
    ///
    /// # Errors
    /// [`Error::Io`] when the file cannot be opened or read, [`Error::UnsupportedFormat`] when no
    /// demuxer or decoder exists for it, [`Error::UnsupportedChannels`] for more than two
    /// channels, [`Error::Corrupt`] when the container is malformed or holds no decodable audio.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })?;
        let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| match e {
                // Running out of bytes while still looking for a container means no container
                // was recognised.
                SymphoniaError::IoError(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
                    Error::UnsupportedFormat {
                        path: path.into(),
                        detail: "no known audio container found".into(),
                    }
                }
                other => map_error(path, other),
            })?;
        let track =
            format
                .default_track(TrackType::Audio)
                .ok_or_else(|| Error::UnsupportedFormat {
                    path: path.into(),
                    detail: "no audio track".into(),
                })?;
        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err(Error::UnsupportedFormat {
                path: path.into(),
                detail: "no audio codec parameters".into(),
            });
        };
        if let Some(channels) = params.channels.as_ref().map(Channels::count) {
            check_channels(path, channels)?;
        }
        let track_id = track.id;
        let total_frames = track.num_frames;
        let delay = track.delay.unwrap_or(0);
        let padding = track.padding.unwrap_or(0);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| map_error(path, e))?;
        tracing::debug!(file = %path.display(), codec = ?params.codec, delay, padding, "decoding");

        let mut this = Self {
            path: path.into(),
            format,
            codec: decoder,
            track_id,
            spec: AudioSpec::CD,
            total_frames,
            delay,
            padding,
            pending: Vec::new(),
            scratch: Vec::new(),
        };
        let Some((rate, channels)) = this.next_block()? else {
            return Err(Error::Corrupt {
                path: path.into(),
                detail: "no decodable audio".into(),
            });
        };
        check_channels(path, channels)?;
        this.spec = AudioSpec::new(rate, u16::try_from(channels).unwrap_or(u16::MAX));
        this.pending = std::mem::take(&mut this.scratch);
        Ok(this)
    }

    /// Sample rate and channel count of the decoded audio.
    #[must_use]
    pub fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Playable frames declared by the container (delay and padding excluded), when known.
    #[must_use]
    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    /// Leading encoder-delay frames the demuxer discards (0 when the format has none).
    #[must_use]
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// Trailing encoder-padding frames the demuxer discards (0 when the format has none).
    #[must_use]
    pub fn padding(&self) -> u32 {
        self.padding
    }

    /// The file being decoded.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delivers every decoded block, interleaved and in file order, and returns the number of
    /// frames delivered. Blocks are whole frames; their size is whatever the codec produces.
    ///
    /// # Errors
    /// Decoding errors as for [`Decoder::open`]; [`Error::Corrupt`] when the stream ends more than
    /// a block short of its declared length (a truncated file); whatever `f` returns.
    pub fn for_each_block(mut self, mut f: impl FnMut(&[f32]) -> Result<()>) -> Result<u64> {
        let channels = u64::from(self.spec.channels);
        let mut delivered = 0_u64;
        let pending = std::mem::take(&mut self.pending);
        if !pending.is_empty() {
            delivered += pending.len() as u64 / channels;
            f(&pending)?;
        }
        while let Some((rate, ch)) = self.next_block()? {
            if rate != self.spec.sample_rate || ch != usize::from(self.spec.channels) {
                return Err(Error::Corrupt {
                    path: self.path.clone(),
                    detail: format!(
                        "stream parameters changed mid-file ({rate} Hz, {ch} ch after {} Hz, {} ch)",
                        self.spec.sample_rate, self.spec.channels
                    ),
                });
            }
            delivered += self.scratch.len() as u64 / channels;
            f(&self.scratch)?;
        }
        if let Some(total) = self.total_frames
            && delivered + TRUNCATION_TOLERANCE_FRAMES < total
        {
            return Err(Error::Corrupt {
                path: self.path.clone(),
                detail: format!("truncated: {delivered} of {total} frames could be decoded"),
            });
        }
        Ok(delivered)
    }

    /// Decodes the next non-empty packet into `scratch`; returns its sample rate and channel
    /// count, or `None` at the end of the stream. Damaged packets are skipped, as symphonia
    /// recommends, and the decoder recovers on the next one.
    fn next_block(&mut self) -> Result<Option<(u32, usize)>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(map_error(&self.path, e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.codec.decode(&packet) {
                Ok(audio) => {
                    if audio.is_empty() {
                        continue;
                    }
                    let rate = audio.spec().rate();
                    let channels = audio.spec().channels().count();
                    self.scratch.clear();
                    audio.copy_to_vec_interleaved(&mut self.scratch);
                    return Ok(Some((rate, channels)));
                }
                Err(SymphoniaError::DecodeError(detail)) => {
                    tracing::warn!(file = %self.path.display(), detail, "skipping undecodable packet");
                }
                Err(SymphoniaError::ResetRequired) => self.codec.reset(),
                Err(e) => return Err(map_error(&self.path, e)),
            }
        }
    }
}

/// Decodes the first audio track of `path` completely.
///
/// # Errors
/// As for [`Decoder::open`] and [`Decoder::for_each_block`].
pub fn read_all(path: &Path) -> Result<AudioBuffer> {
    let decoder = Decoder::open(path)?;
    let spec = decoder.spec();
    let capacity = decoder
        .total_frames()
        .and_then(|n| usize::try_from(n).ok())
        .map_or(0, |n| n.saturating_mul(usize::from(spec.channels)));
    let mut data: Vec<f32> = Vec::with_capacity(capacity);
    decoder.for_each_block(|block| {
        data.extend_from_slice(block);
        Ok(())
    })?;
    Ok(AudioBuffer::new(spec, data))
}

fn check_channels(path: &Path, channels: usize) -> Result<()> {
    match channels {
        1 | 2 => Ok(()),
        0 => Err(Error::Corrupt {
            path: path.into(),
            detail: "zero channels".into(),
        }),
        n => Err(Error::UnsupportedChannels {
            path: path.into(),
            channels: n,
        }),
    }
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
