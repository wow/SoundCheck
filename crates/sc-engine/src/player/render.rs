//! What the player hears, rendered off the audio thread: the open track at its planned gain,
//! with the click on the grid lines, converted to the output device's rate and channels.
//!
//! The mix happens at the track's own rate, so the clicks sit on the grid's sample positions
//! exactly and the resampler moves music and click together. With the click on, the music is
//! 6 dB down so the click cuts through. The converted output is clamped to full scale, so a track
//! whose planned gain leaves overs clips only in the player, never in a file. Clicks go to the
//! first two output channels only, as the music does.

use std::sync::Arc;

use sc_core::analysis::Grid;
use sc_core::{DbFs, Result};
use sc_dsp::StreamResampler;

use super::click::{Accent, Clicks};
use crate::track::Track;

/// Music level while the click plays.
const MUSIC_UNDER_CLICK: f32 = 0.5;

/// A click that started in an earlier block.
#[derive(Debug, Clone, Copy)]
struct Ringing {
    accent: Accent,
    /// Samples of the click already played.
    done: usize,
}

/// Renders the open track, block by block, from a position.
pub struct Renderer {
    track: Arc<Track>,
    rate: u32,
    out_channels: usize,
    position: u64,
    gain: f32,
    grid: Option<Arc<Grid>>,
    click: bool,
    clicks: Clicks,
    ringing: Vec<Ringing>,
    resampler: StreamResampler,
    pcm: Vec<i16>,
    mix: Vec<f32>,
    flushed: bool,
}

impl Renderer {
    /// A renderer of `track` at `gain` (the planned gain, in dB) for an output of `out_rate` Hz
    /// and `out_channels` channels, starting at the first frame with the click off.
    ///
    /// # Errors
    /// [`sc_core::Error::InvalidArgument`] when the rates cannot be converted or a channel
    /// count is zero.
    pub fn new(track: Arc<Track>, gain: DbFs, out_rate: u32, out_channels: u16) -> Result<Self> {
        let rate = track.sample_rate();
        let resampler = StreamResampler::new(rate, out_rate, out_channels)?;
        let mut renderer = Self {
            clicks: Clicks::new(rate),
            track,
            rate,
            out_channels: usize::from(out_channels),
            position: 0,
            gain: 1.0,
            grid: None,
            click: false,
            ringing: Vec::new(),
            resampler,
            pcm: Vec::new(),
            mix: Vec::new(),
            flushed: false,
        };
        renderer.set_gain(gain);
        Ok(renderer)
    }

    /// The next source frame to render.
    #[must_use]
    pub fn position(&self) -> u64 {
        self.position
    }

    /// The track's sample rate, which positions count in.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Continues from source frame `frame`; buffered audio and ringing clicks are dropped.
    pub fn seek(&mut self, frame: u64) {
        self.position = frame;
        self.ringing.clear();
        self.resampler.reset();
        self.flushed = false;
    }

    /// Plays at `gain` (dB) from the next block on.
    pub fn set_gain(&mut self, gain: DbFs) {
        let linear = 10f64.powf(gain.0 / 20.0) / f64::from(self.track.scale());
        #[allow(clippy::cast_possible_truncation)] // a gain in a sane range
        let linear = linear as f32;
        self.gain = linear;
    }

    /// Clicks on `grid`'s lines from the next block on; `None` clicks nowhere.
    pub fn set_grid(&mut self, grid: Option<Arc<Grid>>) {
        self.grid = grid;
    }

    /// Turns the click on or off.
    pub fn set_click(&mut self, on: bool) {
        self.click = on;
        if !on {
            self.ringing.clear();
        }
    }

    /// Everything decodable has been rendered.
    #[must_use]
    pub fn at_end(&self) -> bool {
        self.track.is_done() && self.position >= self.track.decoded_frames()
    }

    /// Renders up to `frames` source frames (fewer where decoding has not got to yet) and
    /// appends the converted output to `out`; returns the source frames rendered. At the end of
    /// the track the converter's last frames are appended once.
    pub fn render(&mut self, frames: usize, out: &mut Vec<f32>) -> usize {
        let from = out.len();
        let n = self.render_block(frames, out);
        if n == 0 && self.at_end() && !self.flushed {
            self.resampler.flush(out);
            self.flushed = true;
        }
        for s in &mut out[from..] {
            *s = s.clamp(-1.0, 1.0);
        }
        n
    }

    fn render_block(&mut self, frames: usize, out: &mut Vec<f32>) -> usize {
        let in_ch = usize::from(self.track.channels());
        self.pcm.resize(frames * in_ch, 0);
        let n = self.track.read(self.position, &mut self.pcm);
        let music = if self.click && self.grid.is_some() {
            self.gain * MUSIC_UNDER_CLICK
        } else {
            self.gain
        };
        let scale = music / 32_768.0;
        let och = self.out_channels;
        self.mix.clear();
        self.mix.resize(n * och, 0.0);
        for (f, frame) in self.pcm[..n * in_ch].chunks_exact(in_ch).enumerate() {
            let out_frame = &mut self.mix[f * och..(f + 1) * och];
            match (in_ch, och) {
                (1, _) => out_frame[..och.min(2)].fill(f32::from(frame[0]) * scale),
                (_, 1) => {
                    out_frame[0] = f32::midpoint(f32::from(frame[0]), f32::from(frame[1])) * scale;
                }
                _ => {
                    out_frame[0] = f32::from(frame[0]) * scale;
                    out_frame[1] = f32::from(frame[1]) * scale;
                }
            }
        }
        if self.click {
            self.add_clicks(n);
        }
        self.resampler.process(&self.mix, out);
        self.position += n as u64;
        n
    }

    /// Adds the clicks sounding in the `n` frames from the current position.
    fn add_clicks(&mut self, n: usize) {
        let mut ringing = std::mem::take(&mut self.ringing);
        for r in &mut ringing {
            r.done += self.mix_click(r.accent, 0, r.done, n);
        }
        let Some(grid) = self.grid.clone() else {
            self.ringing = ringing;
            return;
        };
        let spb = grid.samples_per_beat(self.rate);
        if !(spb.is_finite() && spb > 0.0) {
            self.ringing = ringing;
            return;
        }
        let anchor = sample_f64(grid.anchor.0);
        let start = sample_f64(self.position);
        let bar = i64::from(grid.meter.beats_per_bar.max(1));
        // Lines are rounded to samples: start half a sample early so a line that rounds onto
        // this block's first sample is played here, not lost between two blocks.
        let mut i = ((start - 0.5 - anchor) / spb).ceil();
        loop {
            let line = (anchor + i * spb).round();
            if line >= start + sample_f64(n as u64) {
                break;
            }
            if line >= start {
                let accent = Accent::of(&grid.meter, pulse(i, bar));
                let offset = to_index(line - start);
                let played = self.mix_click(accent, offset, 0, n);
                ringing.push(Ringing {
                    accent,
                    done: played,
                });
            }
            i += 1.0;
        }
        let clicks = &self.clicks;
        ringing.retain(|r| r.done < clicks.sound(r.accent).len());
        self.ringing = ringing;
    }

    /// Adds click samples from `from` onwards at frame `offset` of the block (of `n` frames) to
    /// the first two output channels; returns how many click samples it added.
    fn mix_click(&mut self, accent: Accent, offset: usize, from: usize, n: usize) -> usize {
        let och = self.out_channels;
        let sound = &self.clicks.sound(accent)[from..];
        let count = sound.len().min(n.saturating_sub(offset));
        for (k, &s) in sound[..count].iter().enumerate() {
            let f = offset + k;
            for c in &mut self.mix[f * och..f * och + och.min(2)] {
                *c += s;
            }
        }
        count
    }
}

/// Which pulse of the bar line `i` (counted from bar 1) is.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // an integer-valued index
fn pulse(i: f64, bar: i64) -> usize {
    ((i as i64).rem_euclid(bar)) as usize
}

/// A non-negative sample offset within one block.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_index(x: f64) -> usize {
    x.max(0.0) as usize
}

/// Sample positions of audio in memory are far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn sample_f64(n: u64) -> f64 {
    n as f64
}
