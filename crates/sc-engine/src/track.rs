//! The track open in the grid view: its audio held in memory as 16-bit integers, decoded on a
//! background thread in chunks that the waveform and the click player read while later chunks
//! still decode.
//!
//! A ten-minute stereo track at 44.1 kHz takes 106 MB. Files longer than 20 minutes are folded to
//! mono (a one-hour mix then takes 318 MB instead of 635 MB). A float file with overs is stored
//! scaled down by its sample peak ([`Track::scale`]), so nothing clips in memory. Every chunk
//! carries the minimum and maximum of each 1,024 frames, so a whole-track overview never walks
//! the samples. Dropping the track stops its decoding.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use sc_core::{DbFs, Error, Result};
use sc_io::Decoder;

use crate::cancel::CancelToken;

/// Frames per decoded chunk (about 1.5 s at 44.1 kHz).
pub const CHUNK_FRAMES: usize = 65_536;
/// Frames per summary entry; coarser waveform levels aggregate these.
pub const SUMMARY_FRAMES: usize = 1_024;
/// Finest waveform level, in frames per bin.
pub const MIN_SAMPLES_PER_BIN: u64 = 8;
/// Most bins one waveform request returns (64 KB of min/max pairs).
pub const MAX_BINS: u64 = 16_384;
/// The min/max pair of a bin the decoder has not reached (or could not decode). Samples are
/// stored from -32,767 up, so no audio ever looks like this.
pub const NOT_DECODED: i16 = i16::MIN;
/// Shortest time between two `Decoded` reports.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Files longer than this are held as mono.
pub const MONO_AFTER_S: f64 = 20.0 * 60.0;

/// What the decoding thread reports, in order: `Decoded` at most every 100 ms, then one of
/// `Done` or `Failed`. A track dropped while decoding reports nothing more.
#[derive(Debug)]
pub enum TrackProgress {
    /// Frames decoded so far.
    Decoded(u64),
    /// Decoding finished with this many frames.
    Done(u64),
    /// Decoding stopped; the frames decoded before stay readable.
    Failed {
        /// Why.
        error: Error,
        /// Frames decoded before it stopped.
        frames: u64,
    },
}

/// One decoded chunk: interleaved samples at the stored channel count, and the folded min/max of
/// every [`SUMMARY_FRAMES`] frames.
struct Chunk {
    pcm: Vec<i16>,
    summary: Vec<(i16, i16)>,
}

struct Shared {
    chunks: RwLock<Vec<Arc<Chunk>>>,
    /// Frames in `chunks`; stored after the chunk is pushed.
    decoded: AtomicU64,
    /// Decoding has ended; stored after the last chunk and `failed`.
    done: AtomicBool,
    /// Decoding ended with an error: the frames past `decoded` are unknown, not silent.
    failed: AtomicBool,
    cancel: CancelToken,
}

/// Marks the decoding as ended when the thread leaves, even by a panic in a caller's callback,
/// so readers never wait on a thread that is gone.
struct EndGuard<'a>(&'a Shared);

impl Drop for EndGuard<'_> {
    fn drop(&mut self) {
        if !self.0.done.load(Ordering::Acquire) {
            self.0.failed.store(true, Ordering::Release);
        }
        self.0.done.store(true, Ordering::Release);
    }
}

/// A track decoding into memory.
pub struct Track {
    shared: Arc<Shared>,
    sample_rate: u32,
    channels: u16,
    scale: f32,
    /// A stereo file held as mono.
    folded: bool,
}

impl Track {
    /// Opens `path` and decodes it on a new thread, reporting to `on_progress` from that thread.
    /// `sample_peak` (from the analysis) sets the scale of a file with overs.
    ///
    /// # Errors
    /// As for [`Decoder::open`]: a missing, unreadable or unsupported file.
    pub fn open(
        path: &Path,
        sample_peak: DbFs,
        on_progress: impl Fn(TrackProgress) + Send + 'static,
    ) -> Result<Self> {
        Self::open_with(path, sample_peak, MONO_AFTER_S, on_progress)
    }

    /// As [`Track::open`], folding to mono beyond `mono_after_s` seconds.
    ///
    /// # Errors
    /// As for [`Track::open`].
    pub fn open_with(
        path: &Path,
        sample_peak: DbFs,
        mono_after_s: f64,
        on_progress: impl Fn(TrackProgress) + Send + 'static,
    ) -> Result<Self> {
        let decoder = Decoder::open(path)?;
        let spec = decoder.spec();
        let seconds = decoder
            .total_frames()
            .map_or(0.0, |f| frames_f64(f) / f64::from(spec.sample_rate));
        let channels = if spec.channels == 2 && seconds > mono_after_s {
            1
        } else {
            spec.channels
        };
        let peak = 10f64.powf(sample_peak.0 / 20.0);
        #[allow(clippy::cast_possible_truncation)] // a gain between 0 and 1
        let scale = if peak > 1.0 { (1.0 / peak) as f32 } else { 1.0 };
        let shared = Arc::new(Shared {
            chunks: RwLock::new(Vec::new()),
            decoded: AtomicU64::new(0),
            done: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            cancel: CancelToken::new(),
        });
        let worker = Arc::clone(&shared);
        let fold = (spec.channels, channels);
        std::thread::Builder::new()
            .name("sc-track-decode".into())
            .spawn(move || decode(decoder, &worker, fold, scale, &on_progress))
            .map_err(|e| Error::Internal(format!("cannot start the decoding thread: {e}")))?;
        Ok(Self {
            shared,
            sample_rate: spec.sample_rate,
            channels,
            scale,
            folded: channels != spec.channels,
        })
    }

    /// The file's sample rate.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channels held: the file's, or 1 for a long stereo file.
    #[must_use]
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// A stereo file longer than [`MONO_AFTER_S`] held as mono (left and right averaged).
    #[must_use]
    pub fn folded(&self) -> bool {
        self.folded
    }

    /// Gain the samples were stored with: 1, or below 1 for a float file with overs.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Frames decoded so far.
    #[must_use]
    pub fn decoded_frames(&self) -> u64 {
        self.shared.decoded.load(Ordering::Acquire)
    }

    /// Decoding has finished (completely or not).
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.shared.done.load(Ordering::Acquire)
    }

    /// Decoding stopped on an error; only the first [`Track::decoded_frames`] are known.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }

    fn chunks(&self) -> Vec<Arc<Chunk>> {
        self.shared
            .chunks
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Min/max pairs of `bins` bins of `samples_per_bin` frames from bin `first_bin`, both
    /// channels folded into one lane: `[min0, max0, min1, max1, ...]`. A bin the decoder has not
    /// finished, or could not decode, is `[NOT_DECODED, NOT_DECODED]`; a bin past the end of a
    /// completely decoded file is silence.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when `samples_per_bin` is not a power of two of at least
    /// [`MIN_SAMPLES_PER_BIN`] or `bins` exceeds [`MAX_BINS`].
    pub fn peaks(&self, samples_per_bin: u64, first_bin: u64, bins: u64) -> Result<Vec<i16>> {
        if !samples_per_bin.is_power_of_two() || samples_per_bin < MIN_SAMPLES_PER_BIN {
            return Err(Error::InvalidArgument(format!(
                "{samples_per_bin} frames per bin (a power of two from {MIN_SAMPLES_PER_BIN})"
            )));
        }
        if bins > MAX_BINS {
            return Err(Error::InvalidArgument(format!(
                "{bins} bins (at most {MAX_BINS})"
            )));
        }
        // In this order: every frame counted by `decoded` is then in the chunks copied.
        let done = self.is_done();
        let complete = done && !self.failed();
        let decoded = self.decoded_frames();
        let chunks = self.chunks();
        let mut out = Vec::with_capacity(to_usize(bins * 2));
        for b in first_bin..first_bin.saturating_add(bins) {
            let start = b.saturating_mul(samples_per_bin);
            let end = start.saturating_add(samples_per_bin);
            let (min, max) = if end > decoded && !complete {
                (NOT_DECODED, NOT_DECODED)
            } else if start >= decoded {
                (0, 0)
            } else if samples_per_bin >= SUMMARY_FRAMES as u64 {
                summary_range(&chunks, start, end.min(decoded))
            } else {
                self.pcm_range(&chunks, start, end.min(decoded))
            };
            out.push(min);
            out.push(max);
        }
        Ok(out)
    }

    /// Folded min/max of frames `start..end` from the samples.
    fn pcm_range(&self, chunks: &[Arc<Chunk>], start: u64, end: u64) -> (i16, i16) {
        let ch = usize::from(self.channels);
        let (mut min, mut max) = (i16::MAX, i16::MIN);
        let mut f = start;
        while f < end {
            let (c, offset) = locate(f);
            let Some(chunk) = chunks.get(c) else { break };
            let take = (end - f).min((CHUNK_FRAMES - offset) as u64);
            let slice = &chunk.pcm[offset * ch..(offset + to_usize(take)) * ch];
            for &s in slice {
                min = min.min(s);
                max = max.max(s);
            }
            f += take;
        }
        if min > max { (0, 0) } else { (min, max) }
    }

    /// Copies interleaved frames from `from_frame` into `out` (at [`Track::channels`]) and
    /// returns how many frames it copied: fewer than asked where decoding has not got to yet.
    #[must_use]
    pub fn read(&self, from_frame: u64, out: &mut [i16]) -> usize {
        let ch = usize::from(self.channels);
        let want = (out.len() / ch) as u64;
        let end = from_frame.saturating_add(want).min(self.decoded_frames());
        let chunks = self.chunks();
        let mut f = from_frame;
        let mut written = 0;
        while f < end {
            let (c, offset) = locate(f);
            let Some(chunk) = chunks.get(c) else { break };
            let take = to_usize((end - f).min((CHUNK_FRAMES - offset) as u64));
            out[written * ch..(written + take) * ch]
                .copy_from_slice(&chunk.pcm[offset * ch..(offset + take) * ch]);
            written += take;
            f += take as u64;
        }
        written
    }
}

impl Drop for Track {
    fn drop(&mut self) {
        self.shared.cancel.cancel();
    }
}

/// The decoding thread: converts, folds and summarises each block into chunks.
fn decode(
    decoder: Decoder,
    shared: &Shared,
    (from_ch, to_ch): (u16, u16),
    scale: f32,
    on_progress: &dyn Fn(TrackProgress),
) {
    let guard = EndGuard(shared);
    let mut pcm: Vec<i16> = Vec::with_capacity(CHUNK_FRAMES * usize::from(to_ch));
    let mut total = 0_u64;
    let mut reported = Instant::now();
    let push = |pcm: &mut Vec<i16>, total: &mut u64| {
        let frames = pcm.len() / usize::from(to_ch);
        let chunk = Chunk {
            summary: summarise(pcm, usize::from(to_ch)),
            pcm: std::mem::replace(pcm, Vec::with_capacity(CHUNK_FRAMES * usize::from(to_ch))),
        };
        shared
            .chunks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(chunk));
        *total += frames as u64;
        shared.decoded.store(*total, Ordering::Release);
    };
    let gain = scale * 32_768.0;
    let result = decoder.for_each_block(|block| {
        if shared.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        for frame in block.chunks_exact(usize::from(from_ch)) {
            if to_ch == from_ch {
                pcm.extend(frame.iter().map(|&x| to_i16(x * gain)));
            } else {
                pcm.push(to_i16(
                    frame.iter().sum::<f32>() / f32::from(from_ch) * gain,
                ));
            }
            if pcm.len() == CHUNK_FRAMES * usize::from(to_ch) {
                push(&mut pcm, &mut total);
                if reported.elapsed() >= PROGRESS_INTERVAL {
                    reported = Instant::now();
                    on_progress(TrackProgress::Decoded(total));
                }
            }
        }
        Ok(())
    });
    if !pcm.is_empty() {
        push(&mut pcm, &mut total);
    }
    let report = match result {
        Ok(_) => TrackProgress::Done(total),
        // The track was dropped: nobody is listening.
        Err(Error::Cancelled) if shared.cancel.is_cancelled() => {
            shared.failed.store(true, Ordering::Release);
            drop(guard);
            return;
        }
        Err(e) => {
            shared.failed.store(true, Ordering::Release);
            TrackProgress::Failed {
                error: e,
                frames: total,
            }
        }
    };
    shared.done.store(true, Ordering::Release);
    drop(guard);
    on_progress(report);
}

/// Folded min/max of every [`SUMMARY_FRAMES`] frames of interleaved `pcm`.
fn summarise(pcm: &[i16], channels: usize) -> Vec<(i16, i16)> {
    pcm.chunks(SUMMARY_FRAMES * channels)
        .map(|block| {
            let min = block.iter().copied().min().unwrap_or(0);
            let max = block.iter().copied().max().unwrap_or(0);
            (min, max)
        })
        .collect()
}

/// Folded min/max of frames `start..end` from the summaries (both multiples of
/// [`SUMMARY_FRAMES`], except where `end` is the end of the decoded audio).
fn summary_range(chunks: &[Arc<Chunk>], start: u64, end: u64) -> (i16, i16) {
    let per_chunk = CHUNK_FRAMES / SUMMARY_FRAMES;
    let first = to_usize(start) / SUMMARY_FRAMES;
    let last = to_usize(end).div_ceil(SUMMARY_FRAMES);
    let (mut min, mut max) = (i16::MAX, i16::MIN);
    for s in first..last {
        if let Some(&(lo, hi)) = chunks
            .get(s / per_chunk)
            .and_then(|c| c.summary.get(s % per_chunk))
        {
            min = min.min(lo);
            max = max.max(hi);
        }
    }
    if min > max { (0, 0) } else { (min, max) }
}

/// The chunk holding frame `f` and the frame's offset in it.
fn locate(f: u64) -> (usize, usize) {
    let f = to_usize(f);
    (f / CHUNK_FRAMES, f % CHUNK_FRAMES)
}

/// A sample scaled to 16-bit full scale (32,768), rounded and clamped symmetrically to
/// +/-32,767 so that [`NOT_DECODED`] never occurs in the audio.
#[allow(clippy::cast_possible_truncation)] // clamped into the i16 range first
fn to_i16(x: f32) -> i16 {
    x.round().clamp(-32_767.0, 32_767.0) as i16
}

/// A frame position as an index; positions of audio in memory fit in `usize`.
fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Frame counts are far below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
fn frames_f64(n: u64) -> f64 {
    n as f64
}
