//! The player's live meters: true peak and momentary loudness of the music being heard, per
//! ITU-R BS.1770-5 and EBU R 128 (EBU Tech 3341 "momentary": the K-weighted mean square over the
//! last 400 ms, ungated; true peak per Annex 2, 4x oversampled below 96 kHz), measured with the
//! `ebur128` crate the offline analysis uses, so a reading agrees with the analysis of the same
//! audio.
//!
//! [`Metering`] runs on the feeding thread, never on the audio thread: it takes each block of the
//! track as rendered, at its own rate and channels and at unity gain (before the click is mixed
//! in and before resampling), and after each block stores the block's true peak (the louder
//! channel) and the momentary loudness ending there in a fixed-size history ([`Meters`]). The
//! history is read for the position being heard, which trails the rendering by the queued audio
//! and the device latency; it holds [`HISTORY_BLOCKS`] blocks (6 s at 44.1 kHz, 1.4 s at
//! 192 kHz), more than the 150 ms queue plus the latency of a Bluetooth output.
//!
//! OUT is IN plus the planned gain, added to each reading as it is read: BS.1770 loudness and true
//! peak are homogeneous, so a gain of `g` dB moves both by exactly `g`. A mono file is measured as
//! dual mono, as the analysis does; a track held as mono because it is longer than 20 minutes is
//! measured as that mono fold.
//!
//! Momentary loudness at or below the -70 LUFS absolute gate of BS.1770-5 reads as silence
//! (`None`), as it does in the analysis; so does a block whose samples are all zero, for the peak.
//!
//! A seek or a new track resets the meter: momentary loudness is then `None` until 400 ms of
//! audio have been measured, as the window would otherwise hold silence that is not in the track.

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
        let history = self.lock();
        let readings = &history.readings;
        let i = readings.partition_point(|r| r.end <= heard.0);
        let reading = match readings.get(i) {
            Some(r) if r.start <= heard.0 => *r,
            Some(_) => return None,
            None => {
                let last = *readings.back()?;
                if heard.0 - last.end > BLOCK_FRAMES as u64 {
                    return None;
                }
                last
            }
        };
        Some(frame(reading, history.out_offset_db))
    }

    /// The reading of the last block measured.
    #[must_use]
    pub fn latest(&self) -> Option<MeterFrame> {
        let history = self.lock();
        let last = *history.readings.back()?;
        Some(frame(last, history.out_offset_db))
    }
}

/// The frame for `r` with OUT `offset_db` above IN.
fn frame(r: Reading, offset_db: f64) -> MeterFrame {
    let peak_db = (r.peak > 0.0).then(|| 20.0 * r.peak.log10());
    MeterFrame {
        position: SampleIndex(r.end),
        in_peak: peak_db.map(|p| DbTp(round_db(p))),
        in_momentary: r.momentary.map(|m| Lufs(round_db(m))),
        out_peak: peak_db.map(|p| DbTp(round_db(p + offset_db))),
        out_momentary: r.momentary.map(|m| Lufs(round_db(m + offset_db))),
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
    /// A meter for a track of `rate` Hz and `channels` channels (1 or 2) that stores its
    /// readings in `meters`.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for a channel count other than 1 or 2, or a rate the meter
    /// cannot run at.
    pub fn new(rate: u32, channels: u16, meters: Arc<Meters>) -> Result<Self> {
        if !(1..=2).contains(&channels) {
            return Err(Error::InvalidArgument(format!(
                "{channels} channels; the player's meter takes mono or stereo"
            )));
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

    /// Starts over (a seek or a new track): the filters, the window and the history.
    pub fn reset(&mut self) {
        self.meter.reset();
        self.measured = 0;
        self.meters.clear();
    }
}

#[cfg(test)]
mod tests;
