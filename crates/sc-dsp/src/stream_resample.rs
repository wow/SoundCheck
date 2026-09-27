//! Streaming sample-rate conversion of interleaved audio for playback, with rubato's synchronous
//! FFT resampler (the same converter as [`crate::Resampler`]). Input of any block size is
//! converted in fixed chunks and the output is handed back as soon as it is ready; the
//! converter's start-up delay is skipped after construction and after every [`reset`], so
//! output frame `n` corresponds to input time `n / rate_out` since the last reset. Nothing is
//! trimmed at the end: a player keeps feeding it.
//!
//! [`reset`]: StreamResampler::reset

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Resampler as _};
use sc_core::{Error, Result};

/// Input frames per conversion chunk (23 ms at 44.1 kHz).
pub const STREAM_CHUNK_FRAMES: usize = 1024;

/// An interleaved converter from one fixed rate to another.
pub struct StreamResampler {
    inner: Option<Fft<f32>>,
    channels: usize,
    pending: Vec<f32>,
    scratch: InterleavedOwned<f32>,
    delay: usize,
    skip: usize,
}

impl std::fmt::Debug for StreamResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResampler")
            .field("channels", &self.channels)
            .field("converts", &self.inner.is_some())
            .finish_non_exhaustive()
    }
}

impl StreamResampler {
    /// A converter of `channels` interleaved channels from `rate_in` to `rate_out`; equal rates
    /// copy.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when a rate or the channel count is zero, or rubato refuses
    /// the ratio.
    pub fn new(rate_in: u32, rate_out: u32, channels: u16) -> Result<Self> {
        if rate_in == 0 || rate_out == 0 || channels == 0 {
            return Err(Error::InvalidArgument(
                "sample rates and channels must be positive".into(),
            ));
        }
        let channels = usize::from(channels);
        let (inner, delay, out_max) = if rate_in == rate_out {
            (None, 0, 0)
        } else {
            let fft = Fft::<f32>::new(
                rate_in as usize,
                rate_out as usize,
                STREAM_CHUNK_FRAMES,
                channels,
                FixedSync::Input,
            )
            .map_err(|e| {
                Error::InvalidArgument(format!("resampler {rate_in} -> {rate_out}: {e}"))
            })?;
            let delay = fft.output_delay();
            let out_max = fft.output_frames_max();
            (Some(fft), delay, out_max)
        };
        Ok(Self {
            inner,
            channels,
            pending: Vec::with_capacity(STREAM_CHUNK_FRAMES * channels),
            scratch: InterleavedOwned::new(0.0, channels, out_max),
            delay,
            skip: delay,
        })
    }

    /// Converts interleaved `input` (whole frames), appending every output frame that is ready
    /// to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.inner.is_none() {
            out.extend_from_slice(input);
            return;
        }
        let chunk = STREAM_CHUNK_FRAMES * self.channels;
        let mut rest = input;
        while !rest.is_empty() {
            let take = (chunk - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == chunk {
                self.convert(out);
                self.pending.clear();
            }
        }
    }

    /// Forgets buffered input and the converter's history, as after a seek.
    pub fn reset(&mut self) {
        if let Some(fft) = self.inner.as_mut() {
            fft.reset();
        }
        self.pending.clear();
        self.skip = self.delay;
    }

    /// Converts the one full chunk in `pending`.
    fn convert(&mut self, out: &mut Vec<f32>) {
        let Some(fft) = self.inner.as_mut() else {
            return;
        };
        let ch = self.channels;
        let input = InterleavedSlice::new(&self.pending, ch, STREAM_CHUNK_FRAMES)
            .expect("pending holds exactly one chunk of whole frames");
        let (_, produced) = fft
            .process_into_buffer(&input, &mut self.scratch, None)
            .expect("scratch holds output_frames_max frames");
        let capacity = fft.output_frames_max();
        let scratch = std::mem::replace(&mut self.scratch, InterleavedOwned::new(0.0, ch, 0));
        let data = scratch.take_data();
        let skip = self.skip.min(produced);
        self.skip -= skip;
        out.extend_from_slice(&data[skip * ch..produced * ch]);
        self.scratch =
            InterleavedOwned::new_from(data, ch, capacity).expect("the scratch keeps its size");
    }
}
