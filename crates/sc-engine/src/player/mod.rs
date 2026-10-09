//! The audition player: the open track, original or at its planned gain, with an accented click
//! on the grid and live meters, on the default output device.
//!
//! Four parts, so everything but the device is tested without one:
//! - [`render::Renderer`] mixes and resamples blocks of the track at its own level (any thread);
//! - [`meter::Metering`] measures each rendered block for the IN and OUT meters (the feeding
//!   thread);
//! - [`Feeder`] keeps a lock-free ring about 150 ms ahead of the device and handles play, pause,
//!   seek, the version heard and the monitor volume (the controlling thread);
//! - [`callback::Callback`] copies from the ring into the device buffer and applies the version
//!   gain and the volume (the audio thread, which never allocates, locks or blocks).
//!
//! With the `playback` feature, `Player` runs them on the default cpal device.

pub mod callback;
pub mod click;
#[cfg(feature = "playback")]
mod device;
pub mod meter;
pub mod render;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use sc_core::analysis::Grid;
use sc_core::ipc::{Listen, MeterFrame, Version};
use sc_core::{DbFs, Result, SampleIndex};

pub use callback::{Callback, Shared};
#[cfg(feature = "playback")]
pub use device::{Player, Status};
pub use meter::{Metering, Meters};
pub use render::Renderer;

/// Audio kept queued ahead of the device.
pub const PREROLL_S: f64 = 0.15;
/// Source frames rendered per block (23 ms at 44.1 kHz).
pub const BLOCK_FRAMES: usize = 1024;

/// Where playback is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerState {
    /// Output is running.
    pub playing: bool,
    /// The source frame being heard now.
    pub position: SampleIndex,
    /// Device buffers that found the ring short since loading.
    pub underruns: u64,
}

/// The version gain for `listen` of a track whose planned gain is `planned`: 0 dB for the
/// original, the planned gain for the processed version, and for the level-matched processed
/// version the planned gain less the loudness change it makes. Processing applies gain only, so
/// that change is the planned gain itself and matched versions play at the same level (0 dB).
#[must_use]
pub fn version_gain(listen: Listen, planned: DbFs) -> DbFs {
    match listen.version {
        Version::Original => DbFs(0.0),
        Version::Processed if listen.matched => {
            let loudness_change = planned.0;
            DbFs(planned.0 - loudness_change)
        }
        Version::Processed => planned,
    }
}

/// A gain in dB as the linear factor the callback multiplies by; `None` (muted) is 0.
#[must_use]
pub fn linear(gain: Option<DbFs>) -> f32 {
    #[allow(clippy::cast_possible_truncation)] // a gain factor well inside the f32 range
    gain.map_or(0.0, |g| g.to_linear() as f32)
}

/// The ring for a device of `rate` Hz and `channels` channels: twice the pre-roll.
#[must_use]
pub fn ring(rate: u32, channels: u16) -> (rtrb::Producer<f32>, rtrb::Consumer<f32>) {
    rtrb::RingBuffer::new(preroll_samples(rate, channels) * 2)
}

fn preroll_samples(rate: u32, channels: u16) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a small positive count
    let frames = (PREROLL_S * f64::from(rate)).ceil() as usize;
    frames * usize::from(channels)
}

/// The feeding side of the ring: renders ahead of the callback and applies play, pause, seek
/// and click changes.
pub struct Feeder {
    renderer: Renderer,
    metering: Metering,
    ring: rtrb::Producer<f32>,
    shared: Arc<Shared>,
    planned: DbFs,
    listen: Listen,
    out_rate: u32,
    pending: Vec<f32>,
    preroll: usize,
    /// Source frame of the last seek, which `played` counts from.
    origin: u64,
    /// A seek waiting for the callback to empty the ring.
    awaiting: Option<u64>,
    /// Output stopped by itself at the end of the track.
    finished: bool,
}

impl Feeder {
    /// A feeder for `renderer` into `ring`, played by a device of `out_rate` Hz and
    /// `out_channels` channels, with its meter readings in `meters` (emptied here). It plays the
    /// processed version at a planned gain of 0 dB, at the volume `shared` holds, until told
    /// otherwise.
    ///
    /// # Errors
    /// [`sc_core::Error::InvalidArgument`] when the track's rate or channels cannot be metered.
    pub fn new(
        renderer: Renderer,
        ring: rtrb::Producer<f32>,
        shared: Arc<Shared>,
        meters: Arc<Meters>,
        out_rate: u32,
        out_channels: u16,
    ) -> Result<Self> {
        let origin = renderer.position();
        let mut metering = Metering::new(renderer.sample_rate(), renderer.channels(), meters)?;
        metering.reset();
        let mut feeder = Self {
            renderer,
            metering,
            ring,
            shared,
            planned: DbFs(0.0),
            listen: Listen::default(),
            out_rate,
            pending: Vec::new(),
            preroll: preroll_samples(out_rate, out_channels),
            origin,
            awaiting: None,
            finished: false,
        };
        feeder.set_planned_gain(DbFs(0.0));
        Ok(feeder)
    }

    /// Starts or resumes output.
    pub fn play(&mut self) {
        self.finished = false;
        self.shared.playing.store(true, Ordering::Release);
    }

    fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    /// While paused, the queued audio was rendered with the old settings: render again from
    /// the paused position so playing resumes with the new ones. (While playing, the change is
    /// heard within the queued 150 ms.)
    fn rerender_if_paused(&mut self) {
        let queued = self.ring.slots() < self.ring.buffer().capacity() || !self.pending.is_empty();
        if queued && !self.is_playing() && !self.finished {
            let at = self.state().position.0;
            self.seek(at);
        }
    }

    /// Pauses output; the position holds.
    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Release);
    }

    /// Continues from source frame `frame`: the callback drops what was queued, then the ring
    /// fills from there.
    pub fn seek(&mut self, frame: u64) {
        let epoch = self.shared.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.awaiting = Some(epoch);
        self.pending.clear();
        self.renderer.seek(frame);
        self.metering.reset();
        self.origin = frame;
        self.finished = false;
        self.shared.ended.store(false, Ordering::Release);
    }

    /// The track's planned gain (dB): what the processed version plays at and what OUT reads
    /// above IN. Heard within one device buffer, over [`callback::RAMP_S`].
    pub fn set_planned_gain(&mut self, gain: DbFs) {
        self.planned = gain;
        self.metering.meters().set_out_offset(gain);
        self.apply_listen();
    }

    /// Plays the version `listen` asks for, within one device buffer, over
    /// [`callback::RAMP_S`]; the meters do not change.
    pub fn set_listen(&mut self, listen: Listen) {
        self.listen = listen;
        self.apply_listen();
    }

    /// What is heard.
    #[must_use]
    pub fn listen(&self) -> Listen {
        self.listen
    }

    fn apply_listen(&self) {
        let gain = version_gain(self.listen, self.planned);
        self.shared.set_listen_gain(linear(Some(gain)));
    }

    /// Sets the monitor volume (dB, `None` for muted), applied after the clamp to full scale,
    /// within one device buffer, over [`callback::RAMP_S`]; the meters do not change.
    pub fn set_volume(&mut self, volume: Option<DbFs>) {
        self.shared.set_volume(linear(volume));
    }

    /// The meter reading for the source frame `heard` ([`Meters::at`]).
    #[must_use]
    pub fn meter_at(&self, heard: SampleIndex) -> Option<MeterFrame> {
        self.metering.meters().at(heard)
    }

    /// Clicks on `grid`'s lines from the audio rendered next (about 150 ms later).
    pub fn set_grid(&mut self, grid: Option<Arc<Grid>>) {
        self.renderer.set_grid(grid);
        self.rerender_if_paused();
    }

    /// Turns the click on or off from the audio rendered next.
    pub fn set_click(&mut self, on: bool) {
        self.renderer.set_click(on);
        self.rerender_if_paused();
    }

    /// Queues rendered audio until the pre-roll is full; returns whether it queued anything.
    /// Output that reached the end of the track stops by itself.
    pub fn pump(&mut self) -> bool {
        if let Some(epoch) = self.awaiting {
            if self.shared.acked.load(Ordering::Acquire) < epoch {
                return false;
            }
            self.awaiting = None;
        }
        let mut queued = false;
        loop {
            let buffered = self.ring.buffer().capacity() - self.ring.slots();
            if buffered >= self.preroll {
                break;
            }
            if self.pending.is_empty() {
                let frames = self.renderer.render(BLOCK_FRAMES, &mut self.pending);
                if frames == 0 {
                    if self.renderer.at_end() {
                        self.shared.ended.store(true, Ordering::Release);
                    }
                    break;
                }
                let end = SampleIndex(self.renderer.position());
                self.metering.push(self.renderer.source(), end);
            }
            let take = self.pending.len().min(self.ring.slots());
            if take == 0 {
                break;
            }
            for s in self.pending.drain(..take) {
                // The count was checked against the free slots just above.
                let _ = self.ring.push(s);
            }
            queued = true;
        }
        if self.shared.ended.load(Ordering::Acquire)
            && self.ring.slots() == self.ring.buffer().capacity()
            && self.is_playing()
        {
            self.pause();
            self.finished = true;
        }
        queued
    }

    /// Output stopped by itself at the end of the track (so playing again starts over).
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Where playback is: the frame being heard, from the frames played since the last seek
    /// less the device's latency.
    #[must_use]
    pub fn state(&self) -> PlayerState {
        if self.awaiting.is_some() {
            // Until the callback acknowledges a seek, `played` still counts the old position.
            return PlayerState {
                playing: self.is_playing(),
                position: SampleIndex(self.origin),
                underruns: self.shared.underruns.load(Ordering::Relaxed),
            };
        }
        let played = self.shared.played.load(Ordering::Acquire);
        let latency = self.shared.latency_frames.load(Ordering::Acquire);
        let heard = played.saturating_sub(latency);
        let ratio = f64::from(self.renderer.sample_rate()) / f64::from(self.out_rate);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let advanced = (heard as f64 * ratio).round() as u64;
        PlayerState {
            playing: self.shared.playing.load(Ordering::Acquire),
            position: SampleIndex(self.origin + advanced),
            underruns: self.shared.underruns.load(Ordering::Relaxed),
        }
    }
}
