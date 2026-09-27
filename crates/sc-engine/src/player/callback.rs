//! The audio callback: copies rendered samples from a lock-free ring into the device buffer and
//! nothing else. It never allocates, locks, logs, blocks or panics (a test runs it under an
//! allocation-counting allocator); a short ring outputs silence and counts an underrun.
//!
//! Seeking uses an epoch: the controlling thread raises it, the callback drops everything queued
//! for the old position and acknowledges, and only then is audio for the new position queued,
//! so none of it can be dropped by mistake.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// State shared by the callback and the thread that feeds it.
#[derive(Debug, Default)]
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
}

/// The consuming end, owned by the audio callback.
pub struct Callback {
    ring: rtrb::Consumer<f32>,
    shared: Arc<Shared>,
    channels: usize,
    epoch: u64,
}

impl Callback {
    /// A callback reading `channels`-channel frames from `ring`.
    #[must_use]
    pub fn new(ring: rtrb::Consumer<f32>, shared: Arc<Shared>, channels: u16) -> Self {
        Self {
            ring,
            shared,
            channels: usize::from(channels).max(1),
            epoch: 0,
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
            self.shared.played.store(0, Ordering::Release);
            self.shared.acked.store(epoch, Ordering::Release);
        }
        if !self.shared.playing.load(Ordering::Acquire) {
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
        if copied < out.len() && !self.shared.ended.load(Ordering::Acquire) {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.shared
            .played
            .fetch_add((copied / self.channels) as u64, Ordering::Release);
    }
}
