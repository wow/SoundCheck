//! Sample-rate conversion of a mono stream with rubato's synchronous FFT resampler (band-limited,
//! Blackman-Harris anti-aliasing window), which handles rational ratios such as 48 000 to 22 050
//! exactly. Blocks of any size are pushed; whole chunks are converted as they fill and the tail
//! is flushed by [`Resampler::finish`], which also removes the resampler's start-up delay so
//! that output frame `n` corresponds to input time `n / rate_out`.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};
use sc_core::{Error, Result};

/// Input frames per conversion chunk.
const CHUNK_FRAMES: usize = 1024;

/// A mono resampler from one fixed rate to another.
pub struct Resampler {
    inner: Option<Fft<f32>>,
    rate_in: u32,
    rate_out: u32,
    pending: Vec<f32>,
    output: Vec<f32>,
    scratch: InterleavedOwned<f32>,
    input_frames: usize,
    delay_to_skip: usize,
}

impl std::fmt::Debug for Resampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resampler")
            .field("rate_in", &self.rate_in)
            .field("rate_out", &self.rate_out)
            .field("input_frames", &self.input_frames)
            .finish_non_exhaustive()
    }
}

impl Resampler {
    /// A converter from `rate_in` to `rate_out`; equal rates copy.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when a rate is zero or rubato refuses the ratio.
    pub fn new(rate_in: u32, rate_out: u32) -> Result<Self> {
        if rate_in == 0 || rate_out == 0 {
            return Err(Error::InvalidArgument(
                "sample rates must be positive".into(),
            ));
        }
        let (inner, delay_to_skip, out_max) = if rate_in == rate_out {
            (None, 0, 0)
        } else {
            let fft = Fft::<f32>::new(
                rate_in as usize,
                rate_out as usize,
                CHUNK_FRAMES,
                1,
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
            rate_in,
            rate_out,
            pending: Vec::with_capacity(CHUNK_FRAMES),
            output: Vec::new(),
            scratch: InterleavedOwned::new(0.0, 1, out_max),
            input_frames: 0,
            delay_to_skip,
        })
    }

    /// Input rate in Hz.
    #[must_use]
    pub fn rate_in(&self) -> u32 {
        self.rate_in
    }

    /// Output rate in Hz.
    #[must_use]
    pub fn rate_out(&self) -> u32 {
        self.rate_out
    }

    /// Feeds mono samples; any block size.
    pub fn push(&mut self, mono: &[f32]) {
        self.input_frames += mono.len();
        let Some(_) = self.inner else {
            self.output.extend_from_slice(mono);
            return;
        };
        let mut rest = mono;
        while !rest.is_empty() {
            let need = CHUNK_FRAMES - self.pending.len();
            let take = need.min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == CHUNK_FRAMES {
                let chunk = std::mem::take(&mut self.pending);
                self.convert(&chunk, None);
                self.pending = chunk;
                self.pending.clear();
            }
        }
    }

    /// Flushes the tail and returns the converted signal, trimmed to the exact length
    /// `round(input_frames * rate_out / rate_in)`.
    #[must_use]
    pub fn finish(mut self) -> Vec<f32> {
        if self.inner.is_some() {
            // Pad the last partial chunk with silence, then push enough silence to flush the
            // delay line; the trim below discards whatever exceeds the exact length.
            let partial = self.pending.len();
            let chunk = std::mem::take(&mut self.pending);
            self.convert(&chunk, Some(partial));
            let silence = vec![0.0_f32; CHUNK_FRAMES];
            self.convert(&silence, Some(0));
        }
        let expected = (u128::from(self.input_frames as u64) * u128::from(self.rate_out)
            + u128::from(self.rate_in) / 2)
            / u128::from(self.rate_in);
        let expected = usize::try_from(expected).unwrap_or(usize::MAX);
        self.output.truncate(expected);
        self.output
    }

    /// Converts one input chunk (`partial_len` valid frames, the rest silence) and appends the
    /// output past the start-up delay.
    fn convert(&mut self, chunk: &[f32], partial_len: Option<usize>) {
        let Some(fft) = self.inner.as_mut() else {
            return;
        };
        let mut padded;
        let chunk = if chunk.len() == CHUNK_FRAMES {
            chunk
        } else {
            padded = chunk.to_vec();
            padded.resize(CHUNK_FRAMES, 0.0);
            &padded
        };
        let input = InterleavedSlice::new(chunk, 1, CHUNK_FRAMES)
            .expect("chunk is exactly one channel of CHUNK_FRAMES");
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len,
            active_channels_mask: None,
        };
        let (_, produced) = fft
            .process_into_buffer(&input, &mut self.scratch, Some(&indexing))
            .expect("scratch holds output_frames_max frames of one channel");
        let scratch = std::mem::replace(&mut self.scratch, InterleavedOwned::new(0.0, 1, 0));
        let data = scratch.take_data();
        let skip = self.delay_to_skip.min(produced);
        self.delay_to_skip -= skip;
        self.output.extend_from_slice(&data[skip..produced]);
        self.scratch = InterleavedOwned::new_from(data, 1, self.scratch_capacity())
            .expect("the scratch buffer keeps its size");
    }

    fn scratch_capacity(&self) -> usize {
        self.inner
            .as_ref()
            .map_or(0, rubato::Resampler::output_frames_max)
    }
}

/// Converts a whole mono clip; a convenience over [`Resampler`].
///
/// # Errors
/// As for [`Resampler::new`].
pub fn resample_all(mono: &[f32], rate_in: u32, rate_out: u32) -> Result<Vec<f32>> {
    let mut r = Resampler::new(rate_in, rate_out)?;
    r.push(mono);
    Ok(r.finish())
}
