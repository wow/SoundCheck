//! The player on the default output device (cpal), run by one thread that owns the stream and
//! feeds its ring. Commands arrive over a channel, so every method returns at once; the state
//! is published every few milliseconds.
//!
//! The device's own rate and channel count are used (the track is resampled to them), never
//! changed, so other apps keep theirs. When the stream reports that its device went away, was
//! rerouted or needs rebuilding, playback stops with an error in the state (the way a player
//! pauses when headphones are pulled) and the next `play` opens the default device again. A
//! momentary overload is counted as an underrun and playback goes on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sc_core::analysis::Grid;
use sc_core::{DbFs, Error, Result, SampleIndex};

use super::{Callback, Feeder, PlayerState, Renderer, Shared, ring};
use crate::track::Track;

/// How long the feeding thread sleeps between checks.
const TICK: Duration = Duration::from_millis(5);
/// What the state says after the device went away.
const DEVICE_CHANGED: &str = "Output device changed. Press Space to resume.";

/// What the controlling thread asks for.
enum Command {
    Load { track: Arc<Track>, gain: DbFs },
    Play(Option<u64>),
    Pause,
    Seek(u64),
    Click(bool),
    Grid(Option<Arc<Grid>>),
    Gain(DbFs),
    Unload,
}

/// The state with the last device error, as the UI shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Playing, position and underruns; `None` with no track loaded.
    pub state: Option<PlayerState>,
    /// Why output stopped, when the device failed.
    pub error: Option<String>,
}

/// The click audition player on the default output device.
pub struct Player {
    commands: Option<mpsc::Sender<Command>>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Starts the player's thread; the device opens when a track is loaded.
    ///
    /// # Errors
    /// [`Error::Internal`] when the thread cannot start.
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let published = Arc::clone(&status);
        let thread = std::thread::Builder::new()
            .name("sc-player".into())
            .spawn(move || run(&rx, &published))
            .map_err(|e| Error::Internal(format!("cannot start the player: {e}")))?;
        Ok(Self {
            commands: Some(tx),
            status,
            thread: Some(thread),
        })
    }

    fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            // The thread ends only when the player is dropped.
            let _ = tx.send(command);
        }
    }

    /// Loads `track` to play at `gain` (the planned gain, dB), paused at its start with the
    /// click off.
    pub fn load(&self, track: Arc<Track>, gain: DbFs) {
        self.send(Command::Load { track, gain });
    }

    /// Plays from source frame `from`, or from where it stopped.
    pub fn play(&self, from: Option<SampleIndex>) {
        self.send(Command::Play(from.map(|f| f.0)));
    }

    /// Pauses; the position holds.
    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    /// Moves to source frame `to`, playing or not.
    pub fn seek(&self, to: SampleIndex) {
        self.send(Command::Seek(to.0));
    }

    /// Turns the click on or off.
    pub fn set_click(&self, on: bool) {
        self.send(Command::Click(on));
    }

    /// Clicks on `grid`'s lines (an edit takes effect within about 150 ms).
    pub fn set_grid(&self, grid: Option<Arc<Grid>>) {
        self.send(Command::Grid(grid));
    }

    /// Plays at `gain` (dB).
    pub fn set_gain(&self, gain: DbFs) {
        self.send(Command::Gain(gain));
    }

    /// Stops and releases the track and the device.
    pub fn unload(&self) {
        self.send(Command::Unload);
    }

    /// The latest state.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A loaded track: what the stream needs to be rebuilt after a device error.
struct Loaded {
    track: Arc<Track>,
    gain: DbFs,
    grid: Option<Arc<Grid>>,
    click: bool,
    open: Option<Open>,
    /// Where to continue when the stream is rebuilt.
    resume_at: u64,
}

/// An open stream and its feeder.
struct Open {
    feeder: Feeder,
    /// Set by the stream's error callback.
    failed: Arc<AtomicBool>,
    _stream: cpal::Stream,
}

fn run(commands: &mpsc::Receiver<Command>, status: &Mutex<Status>) {
    let mut loaded: Option<Loaded> = None;
    let mut error: Option<String> = None;
    loop {
        match commands.recv_timeout(TICK) {
            Ok(command) => handle(command, &mut loaded, &mut error),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let Some(l) = loaded.as_mut()
            && let Some(o) = l.open.as_mut()
        {
            l.resume_at = o.feeder.state().position.0;
            if o.failed.load(Ordering::Acquire) {
                l.open = None;
                error = Some(DEVICE_CHANGED.into());
            } else {
                o.feeder.pump();
            }
        }
        let state = loaded.as_ref().map(|l| {
            l.open.as_ref().map_or(
                PlayerState {
                    playing: false,
                    position: SampleIndex(l.resume_at),
                    underruns: 0,
                },
                |o| o.feeder.state(),
            )
        });
        *status.lock().unwrap_or_else(PoisonError::into_inner) = Status {
            state,
            error: error.clone(),
        };
    }
}

fn handle(command: Command, loaded: &mut Option<Loaded>, error: &mut Option<String>) {
    match command {
        Command::Load { track, gain } => {
            *loaded = Some(Loaded {
                track,
                gain,
                grid: None,
                click: false,
                open: None,
                resume_at: 0,
            });
            *error = None;
        }
        Command::Unload => *loaded = None,
        Command::Play(from) => {
            let Some(l) = loaded.as_mut() else { return };
            if l.open.is_none() {
                match open(l) {
                    Ok(o) => {
                        l.open = Some(o);
                        *error = None;
                    }
                    Err(e) => {
                        *error = Some(e.to_string());
                        return;
                    }
                }
            }
            if let Some(o) = l.open.as_mut() {
                if let Some(frame) = from {
                    o.feeder.seek(frame);
                } else if o.feeder.finished() {
                    o.feeder.seek(0);
                }
                o.feeder.play();
            }
        }
        Command::Pause => with_feeder(loaded, |f| f.pause()),
        Command::Seek(frame) => {
            if let Some(l) = loaded.as_mut() {
                l.resume_at = frame;
            }
            with_feeder(loaded, |f| f.seek(frame));
        }
        Command::Click(on) => {
            if let Some(l) = loaded.as_mut() {
                l.click = on;
            }
            with_feeder(loaded, |f| f.set_click(on));
        }
        Command::Grid(grid) => {
            if let Some(l) = loaded.as_mut() {
                l.grid.clone_from(&grid);
            }
            with_feeder(loaded, |f| f.set_grid(grid));
        }
        Command::Gain(gain) => {
            if let Some(l) = loaded.as_mut() {
                l.gain = gain;
            }
            with_feeder(loaded, |f| f.set_gain(gain));
        }
    }
}

fn with_feeder(loaded: &mut Option<Loaded>, f: impl FnOnce(&mut Feeder)) {
    if let Some(open) = loaded.as_mut().and_then(|l| l.open.as_mut()) {
        f(&mut open.feeder);
    }
}

/// Whether a stream error ends playback: the device went away, was rerouted or must be rebuilt,
/// or the host failed. A busy device or a refused real-time promotion only degrades it; an
/// overload is an underrun.
fn stops_playback(kind: cpal::ErrorKind) -> bool {
    !matches!(
        kind,
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceBusy | cpal::ErrorKind::RealtimeDenied
    )
}

/// Opens the default output device for `l`, continuing at `l.resume_at`.
fn open(l: &Loaded) -> Result<Open> {
    let device_error = |e: &dyn std::fmt::Display| Error::Internal(format!("output device: {e}"));
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| Error::Internal("no output device".into()))?;
    let supported = device
        .default_output_config()
        .map_err(|e| device_error(&e))?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(Error::Internal(format!(
            "output device: {} samples are not supported",
            supported.sample_format()
        )));
    }
    let config: cpal::StreamConfig = supported.into();
    let (rate, channels) = (config.sample_rate, config.channels);
    let shared = Arc::new(Shared::default());
    let (producer, consumer) = ring(rate, channels);
    let mut renderer = Renderer::new(Arc::clone(&l.track), l.gain, rate, channels)?;
    renderer.set_grid(l.grid.clone());
    renderer.set_click(l.click);
    renderer.seek(l.resume_at);
    let mut callback = Callback::new(consumer, Arc::clone(&shared), channels);
    let latency = Arc::clone(&shared);
    let overloads = Arc::clone(&shared);
    let failed = Arc::new(AtomicBool::new(false));
    let on_error = Arc::clone(&failed);
    let frame_len = u64::from(channels.max(1));
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let t = info.timestamp();
                if let Some(ahead) = t.playback.checked_duration_since(t.callback) {
                    // The whole buffer counts as played as soon as it is filled; its first
                    // frame is heard `ahead` later.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let frames = (ahead.as_secs_f64() * f64::from(rate)) as u64
                        + data.len() as u64 / frame_len;
                    latency.latency_frames.store(frames, Ordering::Relaxed);
                }
                callback.fill(data);
            },
            move |e: cpal::Error| match e.kind() {
                cpal::ErrorKind::Xrun => {
                    overloads.underruns.fetch_add(1, Ordering::Relaxed);
                }
                kind if stops_playback(kind) => on_error.store(true, Ordering::Release),
                _ => {}
            },
            None,
        )
        .map_err(|e| device_error(&e))?;
    stream.play().map_err(|e| device_error(&e))?;
    let feeder = Feeder::new(renderer, producer, shared, rate, channels);
    Ok(Open {
        feeder,
        failed,
        _stream: stream,
    })
}

#[cfg(test)]
mod tests;
