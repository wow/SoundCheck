//! Decoding a written FLAC file's frames again, with symphonia (a decoder independent of the
//! encoder that wrote them), to verify them: the sample count, the MD5 the STREAMINFO
//! signature must equal (RFC 9639 section 8.2) and a BLAKE3 hash of the same bytes that must
//! equal the hash taken while encoding.
//!
//! symphonia is fed `fLaC`, the STREAMINFO block (flagged last) and the frames, read from the
//! file by range, so what is verified is exactly the bytes on disk, and carried metadata
//! blocks (which symphonia might reject or mis-measure) play no part.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use md5::{Digest, Md5};
use sc_core::{Error, Result};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::default::formats::FlacReader;

use super::streaminfo::STREAMINFO_BYTES;
use super::{le_sample_bytes, stream_head};

/// What a fresh decode of the frames gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedFrames {
    /// Samples per channel.
    pub frames: u64,
    /// MD5 of the samples (interleaved, little-endian, `bits / 8` bytes each).
    pub md5: [u8; 16],
    /// BLAKE3 of the same bytes.
    pub pcm_hash: [u8; 32],
}

/// `head` followed by `range` of `file`, as one seekable stream.
struct Spliced {
    head: Vec<u8>,
    file: File,
    range: Range<u64>,
    pos: u64,
    file_pos: Option<u64>,
}

impl Spliced {
    fn len(&self) -> u64 {
        self.head.len() as u64 + (self.range.end - self.range.start)
    }
}

impl Read for Spliced {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let head = self.head.len() as u64;
        if self.pos < head {
            let at = usize::try_from(self.pos).unwrap_or(usize::MAX);
            let n = (self.head.len() - at).min(buf.len());
            buf[..n].copy_from_slice(&self.head[at..at + n]);
            self.pos += n as u64;
            return Ok(n);
        }
        let left = self.len().saturating_sub(self.pos);
        let want = usize::try_from(left).map_or(buf.len(), |l| l.min(buf.len()));
        if want == 0 {
            return Ok(0);
        }
        let at = self.range.start + (self.pos - head);
        if self.file_pos != Some(at) {
            self.file.seek(SeekFrom::Start(at))?;
        }
        let n = self.file.read(&mut buf[..want])?;
        self.pos += n as u64;
        self.file_pos = Some(at + n as u64);
        Ok(n)
    }
}

impl Seek for Spliced {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len().checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let target = target.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start")
        })?;
        self.pos = target;
        Ok(target)
    }
}

impl MediaSource for Spliced {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}

fn failed(path: &Path, what: &str) -> Error {
    Error::Corrupt {
        path: path.to_path_buf(),
        detail: format!("verification of the written file failed: {what}"),
    }
}

/// Decodes the frames at `frames` of the file `path`, whose STREAMINFO payload is
/// `streaminfo`, expecting `channels` channels of `bits` (16 or 24) bits; `cancel` is checked
/// once per frame.
///
/// # Errors
/// [`Error::Corrupt`] (naming `path`) for a frame that does not decode or parameters that
/// differ; [`Error::Io`] when reading fails; [`Error::Cancelled`].
pub fn decode_frames(
    path: &Path,
    streaminfo: &[u8; STREAMINFO_BYTES],
    frames: Range<u64>,
    channels: u16,
    bits: u16,
    cancel: &AtomicBool,
) -> Result<DecodedFrames> {
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = File::open(path).map_err(io)?;
    let head = stream_head(streaminfo);
    let source = Spliced {
        head,
        file,
        range: frames,
        pos: 0,
        file_pos: None,
    };
    let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
    let symphonia = |e: SymphoniaError| match e {
        SymphoniaError::IoError(source) => io(source),
        other => failed(path, &other.to_string()),
    };
    let mut format = FlacReader::try_new(stream, FormatOptions::default()).map_err(symphonia)?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| failed(path, "no audio track"))?;
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err(failed(path, "no audio parameters"));
    };
    let track_id = track.id;
    let mut codec = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(symphonia)?;
    let shift = 32 - u32::from(bits);
    let (mut md5, mut tee) = (Md5::new(), blake3::Hasher::new());
    let (mut ints, mut bytes) = (Vec::new(), Vec::new());
    let mut decoded = 0_u64;
    loop {
        crate::render::check_cancel(cancel)?;
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(symphonia(e)),
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = codec.decode(&packet).map_err(symphonia)?;
        if audio.spec().channels().count() != usize::from(channels) {
            return Err(failed(path, "the channel count changed"));
        }
        audio.copy_to_vec_interleaved::<i32>(&mut ints);
        for s in &mut ints {
            *s >>= shift;
        }
        le_sample_bytes(&ints, usize::from(bits / 8), &mut bytes);
        md5.update(&bytes);
        tee.update(&bytes);
        decoded += (ints.len() / usize::from(channels.max(1))) as u64;
    }
    Ok(DecodedFrames {
        frames: decoded,
        md5: md5.finalize().into(),
        pcm_hash: *tee.finalize().as_bytes(),
    })
}

#[cfg(test)]
mod tests;
