//! The player's live meters: true peak and momentary loudness of the music being heard, per
//! ITU-R BS.1770-5 and EBU R 128 (EBU Tech 3341 "momentary": the K-weighted mean square over the
//! last 400 ms, ungated; true peak per Annex 2, 4x oversampled below 96 kHz), measured with the
//! `ebur128` crate the offline analysis uses, so a reading agrees with the analysis of the same
//! audio.
//!
//! [`Metering`] runs on the feeding thread, never on the audio thread: it takes each block of the
//! track as rendered, at its own rate and channels and at unity gain (before the click is mixed
//! in and before resampling), and after each block stores the block's true peak (the loudest
//! channel) and the momentary loudness ending there in a fixed-size history ([`Meters`]). The
//! history is read for the position being heard, which trails the rendering by the queued audio
//! and the device latency; it holds [`HISTORY_BLOCKS`] blocks (6 s at 44.1 kHz, 1.4 s at
//! 192 kHz), more than the 150 ms queue plus the latency of a Bluetooth output. A reader that
//! looks a few times a second ([`Meters::since`]) gets the largest block peak since its last
//! reading (or, for its first, since the start, seek or load), so a short over between two
//! readings is never lost.
//!
//! OUT is IN plus the planned gain, added to each reading as it is read: BS.1770 loudness and true
//! peak are homogeneous, so a gain of `g` dB moves both by exactly `g`. A mono file is measured as
//! dual mono, as the analysis does; more channels use the BS.1770 channel order (L, R, C, LFE
//! excluded, Ls, Rs). A stereo track held as mono because it is longer than 20 minutes is
//! measured as that mono fold, and its frames say so ([`MeterFrame::folded`]).
//!
//! Momentary loudness at or below the -70 LUFS absolute gate of BS.1770-5 reads as silence
//! (`None`), as it does in the analysis; so does a block whose samples are all zero, for the peak.
//!
//! A seek or a new track resets the meter: momentary loudness is then `None` until 400 ms of
//! audio have been measured, as the window would otherwise hold silence that is not in the track.
//! The reset also empties the filters, so before measuring again the meter is primed with the
//! 100 ms of the track just before the new position ([`Metering::prime`]); they warm up the
//! K-weighting filters and the true-peak interpolator without being measured. Unprimed, the
//! interpolator (a 48-tap FIR, 12 taps per phase at 4x and 24 at 2x) would read the first frames
//! after a seek against zeros that are not in the track, and on a bass note near full scale that
//! step overshoots the track's true peak by up to about 1 dB: a false over. Primed, every point
//! it reads is interpolated from the track's own frames, as in a continuous play, and the
//! K-weighting filters enter the 400 ms window in the state a continuous play leaves them in, to
//! about 1e-9 (their slowest poles, the 38 Hz high-pass, decay that far in 100 ms). At the start
//! of the track there is nothing to prime with, as for the analysis, which starts from silence.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use ebur128::{Channel, EbuR128, Mode};
use sc_core::ipc::MeterFrame;
use sc_core::{DbFs, DbTp, Error, Lufs, Result, SampleIndex};

use super::BLOCK_FRAMES;

/// The absolute gate of BS.1770-5, LUFS: momentary loudness at or below it reads as silence.
pub const SILENCE_LUFS: f64 = -70.0;

/// Blocks kept in the history.
pub const HISTORY_BLOCKS: usize = 256;

/// One measured block: frames `start..end` of the track.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reading {
    start: u64,
    end: u64,
    /// Largest true peak of the block over the channels, linear (0 for digital silence).
    peak: f64,
    /// Momentary loudness ending at `end`, LUFS; `None` below the absolute gate.
    momentary: Option<f64>,
}

#[derive(Debug)]
struct History {
    readings: VecDeque<Reading>,
    /// The planned gain, dB, which OUT adds to IN.
    out_offset_db: f64,
    /// The readings are of a stereo track folded to mono.
    folded: bool,
}

/// The meter readings of the blocks rendered last, shared between the feeding thread (which
/// writes them) and whoever reports them (which reads them). Neither side is the audio thread.
#[derive(Debug)]
pub struct Meters {
    history: Mutex<History>,
}

impl Default for Meters {
    fn default() -> Self {
        Self::new()
    }
}

impl Meters {
    /// An empty history with OUT at 0 dB from IN.
    #[must_use]
    pub fn new() -> Self {
        Self {
            history: Mutex::new(History {
                readings: VecDeque::with_capacity(HISTORY_BLOCKS),
                out_offset_db: 0.0,
                folded: false,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, History> {
        self.history.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// OUT reads `gain` (the planned gain, dB) above IN from now on, for readings already stored
    /// too.
    pub fn set_out_offset(&self, gain: DbFs) {
        self.lock().out_offset_db = gain.0;
    }

    /// Marks the readings as those of a stereo track held (and measured) as mono.
    pub fn set_folded(&self, folded: bool) {
        self.lock().folded = folded;
    }

    /// Forgets every reading (a seek or a new track).
    pub fn clear(&self) {
        self.lock().readings.clear();
    }

    /// Stores a reading, dropping the oldest when the history is full (never reallocating).
    fn push(&self, reading: Reading) {
        let mut history = self.lock();
        if history.readings.len() == HISTORY_BLOCKS {
            history.readings.pop_front();
        }
        history.readings.push_back(reading);
    }

    /// The reading for the source frame `heard`: the block that holds it, or the last block when
    /// `heard` is at most one block past it (the end of the track). `None` when no stored block
    /// is that close (nothing measured yet since a seek, or the history has moved on).
    #[must_use]
    pub fn at(&self, heard: SampleIndex) -> Option<MeterFrame> {
        self.read(heard, |_, i| i)
    }

    /// As [`Meters::at`], but the peaks are the largest of every stored block from the one after
    /// the block ending at `previous` (the `position` of the frame read before) up to the one
    /// holding `heard`: what a reader that looks every 33 ms needs so that no block's peak is
    /// skipped. With no `previous`, or one at or after the heard block (a seek back or a replay
    /// from the end), the peaks are those of every stored block up to the heard one: a seek or a
    /// load empties the history, so that is everything played since. Momentary loudness is
    /// always the heard block's.
    #[must_use]
    pub fn since(&self, heard: SampleIndex, previous: Option<SampleIndex>) -> Option<MeterFrame> {
        self.read(heard, |readings, i| match previous {
            Some(p) if p.0 < readings[i].end => readings.partition_point(|r| r.end <= p.0).min(i),
            _ => 0,
        })
    }

    /// The frame for the block holding `heard`, with the largest peak of the stored blocks from
    /// `first(readings, heard_index)` up to it.
    fn read(
        &self,
        heard: SampleIndex,
        first: impl FnOnce(&VecDeque<Reading>, usize) -> usize,
    ) -> Option<MeterFrame> {
        let history = self.lock();
        let readings = &history.readings;
        let i = readings.partition_point(|r| r.end <= heard.0);
        let i = match readings.get(i) {
            Some(r) if r.start <= heard.0 => i,
            Some(_) => return None,
            None => {
                let last = readings.back()?;
                if heard.0 - last.end > BLOCK_FRAMES as u64 {
                    return None;
                }
                readings.len() - 1
            }
        };
        let mut reading = readings[i];
        for r in readings.range(first(readings, i)..i) {
            reading.peak = reading.peak.max(r.peak);
        }
        Some(frame(reading, history.out_offset_db, history.folded))
    }

    /// The reading of the last block measured.
    #[must_use]
    pub fn latest(&self) -> Option<MeterFrame> {
        let history = self.lock();
        let last = *history.readings.back()?;
        Some(frame(last, history.out_offset_db, history.folded))
    }
}

/// The frame for `r` with OUT `offset_db` above IN.
fn frame(r: Reading, offset_db: f64, folded: bool) -> MeterFrame {
    let peak_db = (r.peak > 0.0).then(|| 20.0 * r.peak.log10());
    MeterFrame {
        position: SampleIndex(r.end),
        in_peak: peak_db.map(|p| DbTp(round_db(p))),
        in_momentary: r.momentary.map(|m| Lufs(round_db(m))),
        out_peak: peak_db.map(|p| DbTp(round_db(p + offset_db))),
        out_momentary: r.momentary.map(|m| Lufs(round_db(m + offset_db))),
        folded,
    }
}

/// A level rounded to 0.001 dB: finer than any meter shows, and short on the wire.
fn round_db(db: f64) -> f64 {
    (db * 1000.0).round() / 1000.0
}

/// Measures the blocks the player renders and stores their readings in [`Meters`].
pub struct Metering {
    meter: EbuR128,
    channels: usize,
    /// Frames in the 400 ms momentary window.
    window_frames: u64,
    /// Frames measured since the last reset, up to `window_frames`.
    measured: u64,
    meters: Arc<Meters>,
}

impl std::fmt::Debug for Metering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metering")
            .field("channels", &self.channels)
            .field("measured", &self.measured)
            .finish_non_exhaustive()
    }
}

impl Metering {
    /// A meter for a track of `rate` Hz and `channels` channels that stores its readings in
    /// `meters`.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for no channels, or a rate the meter cannot run at.
    pub fn new(rate: u32, channels: u16, meters: Arc<Meters>) -> Result<Self> {
        if channels == 0 {
            return Err(Error::InvalidArgument("no channels to meter".into()));
        }
        let mut meter = EbuR128::new(u32::from(channels), rate, Mode::M | Mode::TRUE_PEAK)
            .map_err(|e| Error::InvalidArgument(format!("meter at {rate} Hz: {e}")))?;
        if channels == 1 {
            meter
                .set_channel(0, Channel::DualMono)
                .map_err(|e| Error::InvalidArgument(format!("dual mono: {e}")))?;
        }
        Ok(Self {
            meter,
            channels: usize::from(channels),
            // The meter's own window: four 100 ms blocks of `(rate + 5) / 10` frames.
            window_frames: (u64::from(rate) + 5) / 10 * 4,
            measured: 0,
            meters,
        })
    }

    /// The history this meter writes to.
    #[must_use]
    pub fn meters(&self) -> &Arc<Meters> {
        &self.meters
    }

    /// Measures `block` (interleaved, whole frames, at unity gain), the frames of the track that
    /// end just before frame `end`, and stores the reading.
    pub fn push(&mut self, block: &[f32], end: SampleIndex) {
        let frames = (block.len() / self.channels) as u64;
        if frames == 0 || self.meter.add_frames_f32(block).is_err() {
            return;
        }
        self.measured = (self.measured + frames).min(self.window_frames);
        let peak = (0..self.channels)
            .filter_map(|c| u32::try_from(c).ok())
            .map(|c| self.meter.prev_true_peak(c).unwrap_or(0.0))
            .fold(0.0_f64, f64::max);
        let momentary = if self.measured < self.window_frames {
            None
        } else {
            self.meter
                .loudness_momentary()
                .ok()
                .filter(|&m| m > SILENCE_LUFS)
        };
        self.meters.push(Reading {
            start: end.0.saturating_sub(frames),
            end: end.0,
            peak,
            momentary,
        });
    }

    /// Starts over (a seek or a new track): the filters, the window and the history. Prime the
    /// meter ([`Metering::prime`]) before measuring the first block after the new position.
    pub fn reset(&mut self) {
        self.meter.reset();
        self.measured = 0;
        self.meters.clear();
    }

    /// Frames before a new position that [`Metering::prime`] uses: 100 ms of the track, far more
    /// than the true-peak interpolator holds (12 frames at 4x, 24 at 2x) and enough for the
    /// K-weighting filters to settle.
    #[must_use]
    pub fn prime_frames(&self) -> usize {
        // `window_frames` is four 100 ms blocks.
        usize::try_from(self.window_frames / 4).unwrap_or(usize::MAX)
    }

    /// After a [`Metering::reset`], warms the filters with `preceding`: the frames of the track
    /// that come just before the first block measured next (interleaved, whole frames, at unity
    /// gain; empty at the start of the track). Only the last [`Metering::prime_frames`] are used.
    /// They are not measured: no peak, no reading, and the 400 ms window stays empty, so
    /// momentary loudness still waits for 400 ms of measured audio.
    pub fn prime(&mut self, preceding: &[f32]) {
        let frames = preceding.len() / self.channels;
        let used = frames.min(self.prime_frames());
        let tail = &preceding[(frames - used) * self.channels..frames * self.channels];
        if !tail.is_empty() {
            // Fails only for a length that is not whole frames, which `tail` always is.
            let _ = self.meter.seed_frames_f32(tail);
        }
    }
}

#[cfg(test)]
mod tests;
