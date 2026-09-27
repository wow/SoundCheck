//! The click audition player: the open track at its planned gain with an accented click on the
//! grid, on the default output device.
//!
//! Three parts, so everything but the device is tested without one:
//! - [`render::Renderer`] mixes and resamples blocks of the track (any thread);
//! - [`Feeder`] keeps a lock-free ring about 150 ms ahead of the device and handles play, pause
//!   and seek (the controlling thread);
//! - [`callback::Callback`] copies from the ring into the device buffer (the audio thread, which
//!   never allocates, locks or blocks).
//!
//! With the `playback` feature, `Player` runs them on the default cpal device.

pub mod callback;
pub mod click;
#[cfg(feature = "playback")]
mod device;
pub mod render;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use sc_core::analysis::Grid;
use sc_core::{DbFs, SampleIndex};

pub use callback::{Callback, Shared};
#[cfg(feature = "playback")]
pub use device::Player;
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
    ring: rtrb::Producer<f32>,
    shared: Arc<Shared>,
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
    /// `out_channels` channels.
    #[must_use]
    pub fn new(
        renderer: Renderer,
        ring: rtrb::Producer<f32>,
        shared: Arc<Shared>,
        out_rate: u32,
        out_channels: u16,
    ) -> Self {
        let origin = renderer.position();
        Self {
            renderer,
            ring,
            shared,
            out_rate,
            pending: Vec::new(),
            preroll: preroll_samples(out_rate, out_channels),
            origin,
            awaiting: None,
            finished: false,
        }
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
        self.origin = frame;
        self.finished = false;
        self.shared.ended.store(false, Ordering::Release);
    }

    /// Plays at `gain` (dB) from the audio rendered next.
    pub fn set_gain(&mut self, gain: DbFs) {
        self.renderer.set_gain(gain);
        self.rerender_if_paused();
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
