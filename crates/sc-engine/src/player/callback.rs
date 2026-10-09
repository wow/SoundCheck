//! The audio callback: copies rendered samples from a lock-free ring into the device buffer and
//! applies the listening gains. It never allocates, locks, logs, blocks or panics (a test runs it
//! under an allocation-counting allocator); a ring that runs short once audio is flowing outputs
//! silence and counts an underrun (the wait for the first audio after a start or a seek is not
//! one).
//!
//! Two gains are applied here rather than when rendering, so a change is heard within one device
//! buffer instead of after the 150 ms of queued audio: the version gain (0 dB for the original,
//! the planned gain for the processed version), then the clamp to full scale, then the monitor
//! volume. Clamping before the volume means turning the volume down never hides that the
//! processed version clips in the player. Each gain moves to a new value along a straight line
//! over 10 ms of output, so switching versions or dragging the volume does not click. The click
//! is in the rendered audio, so it follows the version gain and stays 6 dB over the music.
//!
//! Seeking uses an epoch: the controlling thread raises it, the callback drops everything queued
//! for the old position and acknowledges, and only then is audio for the new position queued,
//! so none of it can be dropped by mistake.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// How long a gain change takes, in seconds of output.
pub const RAMP_S: f64 = 0.010;

/// State shared by the callback and the thread that feeds it.
#[derive(Debug)]
pub struct Shared {
    /// Raised by the feeding thread on every seek.
    pub epoch: AtomicU64,
    /// The epoch the callback has emptied the ring for.
    pub acked: AtomicU64,
    /// Output is running; paused output is silence and consumes nothing.
    pub playing: AtomicBool,
    /// The feeding thread has queued the last audio there is.
    pub ended: AtomicBool,
    /// Output frames played since the last seek.
    pub played: AtomicU64,
    /// Device buffers that found the ring short while playing.
    pub underruns: AtomicU64,
    /// Frames between the callback and the moment its buffer is heard, as the device reports.
    pub latency_frames: AtomicU64,
    /// The version gain as a linear factor, as `f32` bits ([`Shared::set_listen_gain`]).
    listen_gain: AtomicU32,
    /// The monitor volume as a linear factor (0 is muted), as `f32` bits
    /// ([`Shared::set_volume`]).
    volume: AtomicU32,
}

impl Default for Shared {
    /// Stopped, at unity gain and full volume.
    fn default() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            acked: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            played: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
            listen_gain: AtomicU32::new(1.0_f32.to_bits()),
            volume: AtomicU32::new(1.0_f32.to_bits()),
        }
    }
}

impl Shared {
    /// Plays the rendered audio at `linear` times its level (the version gain), before the clamp
    /// to full scale; the callback ramps to it over [`RAMP_S`].
    pub fn set_listen_gain(&self, linear: f32) {
        self.listen_gain.store(linear.to_bits(), Ordering::Release);
    }

    /// The version gain the callback is moving to, linear.
    #[must_use]
    pub fn listen_gain(&self) -> f32 {
        f32::from_bits(self.listen_gain.load(Ordering::Acquire))
    }

    /// Scales what is heard by `linear` (the monitor volume, 0 for muted), after the clamp; the
    /// callback ramps to it over [`RAMP_S`].
    pub fn set_volume(&self, linear: f32) {
        self.volume.store(linear.to_bits(), Ordering::Release);
    }

    /// The monitor volume the callback is moving to, linear.
    #[must_use]
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Acquire))
    }
}

/// A gain moving to its target along a straight line, one step per output frame.
#[derive(Debug, Clone, Copy)]
struct Ramp {
    /// Where the current line starts.
    from: f64,
    target: f32,
    /// Change per frame.
    step: f64,
    /// Frames of the line played so far, and its length.
    done: u32,
    frames: u32,
    /// The gain of the last frame.
    current: f32,
}

impl Ramp {
    fn at(gain: f32) -> Self {
        Self {
            from: f64::from(gain),
            target: gain,
            step: 0.0,
            done: 0,
            frames: 0,
            current: gain,
        }
    }

    /// Heads for `target` over `frames` frames from the gain now, when it is a new target.
    fn retarget(&mut self, target: f32, frames: u32) {
        if target.to_bits() == self.target.to_bits() {
            return;
        }
        self.target = target;
        self.from = f64::from(self.current);
        self.step = (f64::from(target) - self.from) / f64::from(frames);
        self.done = 0;
        self.frames = frames;
    }

    /// Jumps to the target (nothing is playing, so nothing can click).
    fn settle(&mut self) {
        *self = Self::at(self.target);
    }

    /// The gain of the next frame.
    fn next(&mut self) -> f32 {
        if self.done < self.frames {
            self.done += 1;
            #[allow(clippy::cast_possible_truncation)] // a gain between two f32 gains
            let value = (self.from + self.step * f64::from(self.done)) as f32;
            self.current = if self.done == self.frames {
                self.target
            } else {
                value
            };
        }
        self.current
    }
}

/// The consuming end, owned by the audio callback.
pub struct Callback {
    ring: rtrb::Consumer<f32>,
    shared: Arc<Shared>,
    channels: usize,
    epoch: u64,
    /// Audio has arrived since the last seek.
    primed: bool,
    /// Frames a gain change takes ([`RAMP_S`] at the device's rate).
    ramp_frames: u32,
    listen: Ramp,
    volume: Ramp,
}

impl Callback {
    /// A callback reading `channels`-channel frames from `ring` for a device of `rate` Hz,
    /// starting at the gains `shared` holds.
    #[must_use]
    pub fn new(ring: rtrb::Consumer<f32>, shared: Arc<Shared>, channels: u16, rate: u32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 10 ms of frames
        let ramp_frames = ((RAMP_S * f64::from(rate)).round() as u32).max(1);
        let (listen, volume) = (Ramp::at(shared.listen_gain()), Ramp::at(shared.volume()));
        Self {
            ring,
            shared,
            channels: usize::from(channels).max(1),
            epoch: 0,
            primed: false,
            ramp_frames,
            listen,
            volume,
        }
    }

    /// Fills `out` (interleaved device samples).
    pub fn fill(&mut self, out: &mut [f32]) {
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if epoch != self.epoch {
            let stale = self.ring.slots();
            if let Ok(chunk) = self.ring.read_chunk(stale) {
                chunk.commit_all();
            }
            self.epoch = epoch;
            self.primed = false;
            self.shared.played.store(0, Ordering::Release);
            self.shared.acked.store(epoch, Ordering::Release);
        }
        self.listen
            .retarget(self.shared.listen_gain(), self.ramp_frames);
        self.volume.retarget(self.shared.volume(), self.ramp_frames);
        if !self.shared.playing.load(Ordering::Acquire) {
            self.listen.settle();
            self.volume.settle();
            out.fill(0.0);
            return;
        }
        let available = self.ring.slots().min(out.len());
        let take = available - available % self.channels;
        let mut copied = 0;
        if let Ok(chunk) = self.ring.read_chunk(take) {
            let (first, second) = chunk.as_slices();
            out[..first.len()].copy_from_slice(first);
            out[first.len()..first.len() + second.len()].copy_from_slice(second);
            copied = first.len() + second.len();
            chunk.commit_all();
        }
        out[copied..].fill(0.0);
        if copied < out.len() && self.primed && !self.shared.ended.load(Ordering::Acquire) {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.primed |= copied > 0;
        for frame in out.chunks_exact_mut(self.channels) {
            let (gain, volume) = (self.listen.next(), self.volume.next());
            for s in frame {
                *s = (*s * gain).clamp(-1.0, 1.0) * volume;
            }
        }
        self.shared
            .played
            .fetch_add((copied / self.channels) as u64, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
