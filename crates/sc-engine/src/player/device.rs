//! The player on the default output device (cpal), run by one thread that owns the stream and
//! feeds its ring. Commands arrive over a channel, so every method returns at once; the state
//! is published every few milliseconds.
//!
//! The device's own rate and channel count are used (the track is resampled to them), never
//! changed, so other apps keep theirs. When the stream reports that its device went away, was
//! rerouted or needs rebuilding, playback stops with an error in the state (the way a player
//! pauses when headphones are pulled) and the next `play` opens the default device again. A
//! momentary overload is counted as an underrun and playback goes on.
//!
//! The version heard and the monitor volume live in the player, not in the stream: a rebuilt
//! stream starts with them, the version goes back to the processed one when a track loads, and
//! the volume stays as it was for every track.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sc_core::analysis::Grid;
use sc_core::ipc::{Listen, MeterFrame};
use sc_core::{DbFs, Error, Result, SampleIndex};

use super::{Callback, Feeder, Meters, PlayerState, Renderer, Shared, ring};
use crate::track::Track;

/// How long the feeding thread sleeps between checks.
const TICK: Duration = Duration::from_millis(5);
/// What the state says after the device went away.
const DEVICE_CHANGED: &str = "Output device changed. Press Space to resume.";

/// What the controlling thread asks for.
enum Command {
    Load {
        track: Arc<Track>,
        gain: DbFs,
        load: u64,
    },
    Play(Option<u64>),
    Pause,
    Seek(u64),
    Click(bool),
    Grid(Option<Arc<Grid>>),
    PlannedGain(DbFs),
    Listen(Listen),
    Volume(Option<DbFs>),
    Unload,
}

/// The state with the last device error, as the UI shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// Playing, position and underruns; `None` with no track loaded.
    pub state: Option<PlayerState>,
    /// Why output stopped, when the device failed.
    pub error: Option<String>,
    /// The number [`Player::load`] returned for the track this status describes (0 before the
    /// first load). Right after a load, the status still describes the track before until the
    /// player's thread has taken the new one.
    pub load: u64,
    /// The meter reading for `state`'s position while playing; `None` while stopped.
    pub meter: Option<MeterFrame>,
    /// The version heard.
    pub listen: Listen,
}

/// The click audition player on the default output device.
pub struct Player {
    commands: Option<mpsc::Sender<Command>>,
    status: Arc<Mutex<Status>>,
    meters: Arc<Meters>,
    thread: Option<JoinHandle<()>>,
    /// Loads so far.
    loads: AtomicU64,
}

impl Player {
    /// Starts the player's thread; the device opens when a track is loaded.
    ///
    /// # Errors
    /// [`Error::Internal`] when the thread cannot start.
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let meters = Arc::new(Meters::new());
        let published = Arc::clone(&status);
        let mut thread = Thread::new(Arc::clone(&meters));
        let thread = std::thread::Builder::new()
            .name("sc-player".into())
            .spawn(move || thread.run(&rx, &published))
            .map_err(|e| Error::Internal(format!("cannot start the player: {e}")))?;
        Ok(Self {
            commands: Some(tx),
            status,
            meters,
            thread: Some(thread),
            loads: AtomicU64::new(0),
        })
    }

    fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            // The thread ends only when the player is dropped.
            let _ = tx.send(command);
        }
    }

    /// Loads `track` with its planned gain `gain` (dB), paused at its start with the click off,
    /// playing the processed version (at the volume set before). Returns the load's number, which
    /// [`Status::load`] carries once the player's thread has taken the track.
    pub fn load(&self, track: Arc<Track>, gain: DbFs) -> u64 {
        let load = self.loads.fetch_add(1, Ordering::AcqRel) + 1;
        self.send(Command::Load { track, gain, load });
        load
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

    /// Changes the track's planned gain (dB, after a target change): the processed version plays
    /// at it and OUT reads it above IN.
    pub fn set_planned_gain(&self, gain: DbFs) {
        self.send(Command::PlannedGain(gain));
    }

    /// Plays the version `listen` asks for, at the same position; heard within one device
    /// buffer after a 10 ms ramp.
    pub fn set_listen(&self, listen: Listen) {
        self.send(Command::Listen(listen));
    }

    /// Sets the monitor volume (dB, at most 0; `None` mutes) for this and every later track.
    /// It scales only what is heard: the meters and the planned gain do not change.
    pub fn set_volume(&self, volume: Option<DbFs>) {
        self.send(Command::Volume(volume));
    }

    /// The meter reading for the source frame `heard`, as the history holds it
    /// ([`Meters::at`]); [`Status::meter`] is this for the published position.
    #[must_use]
    pub fn meter_at(&self, heard: SampleIndex) -> Option<MeterFrame> {
        self.meters.at(heard)
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
    listen: Listen,
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

/// What the player's thread holds between commands.
struct Thread {
    loaded: Option<Loaded>,
    error: Option<String>,
    /// The number of the track loaded last.
    load: u64,
    /// The monitor volume, kept across tracks.
    volume: Option<DbFs>,
    meters: Arc<Meters>,
}

impl Thread {
    fn new(meters: Arc<Meters>) -> Self {
        Self {
            loaded: None,
            error: None,
            load: 0,
            volume: Some(DbFs(0.0)),
            meters,
        }
    }

    fn run(&mut self, commands: &mpsc::Receiver<Command>, status: &Mutex<Status>) {
        loop {
            match commands.recv_timeout(TICK) {
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if let Some(l) = self.loaded.as_mut()
                && let Some(o) = l.open.as_mut()
            {
                l.resume_at = o.feeder.state().position.0;
                if o.failed.load(Ordering::Acquire) {
                    l.open = None;
                    self.error = Some(DEVICE_CHANGED.into());
                } else {
                    o.feeder.pump();
                }
            }
            *status.lock().unwrap_or_else(PoisonError::into_inner) = self.status();
        }
    }

    fn status(&self) -> Status {
        let state = self.loaded.as_ref().map(|l| {
            l.open.as_ref().map_or(
                PlayerState {
                    playing: false,
                    position: SampleIndex(l.resume_at),
                    underruns: 0,
                },
                |o| o.feeder.state(),
            )
        });
        let meter = self
            .loaded
            .as_ref()
            .and_then(|l| l.open.as_ref().zip(state))
            .filter(|(_, s)| s.playing)
            .and_then(|(o, s)| o.feeder.meter_at(s.position));
        Status {
            state,
            error: self.error.clone(),
            load: self.load,
            meter,
            listen: self.loaded.as_ref().map(|l| l.listen).unwrap_or_default(),
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Load { track, gain, load } => {
                self.load = load;
                self.meters.clear();
                self.meters.set_out_offset(gain);
                self.loaded = Some(Loaded {
                    track,
                    gain,
                    grid: None,
                    click: false,
                    listen: Listen::default(),
                    open: None,
                    resume_at: 0,
                });
                self.error = None;
            }
            Command::Unload => {
                self.loaded = None;
                self.meters.clear();
            }
            Command::Play(from) => self.play(from),
            Command::Pause => self.with_feeder(|f| f.pause()),
            Command::Seek(frame) => {
                if let Some(l) = self.loaded.as_mut() {
                    l.resume_at = frame;
                }
                self.with_feeder(|f| f.seek(frame));
            }
            Command::Click(on) => {
                if let Some(l) = self.loaded.as_mut() {
                    l.click = on;
                }
                self.with_feeder(|f| f.set_click(on));
            }
            Command::Grid(grid) => {
                if let Some(l) = self.loaded.as_mut() {
                    l.grid.clone_from(&grid);
                }
                self.with_feeder(|f| f.set_grid(grid));
            }
            Command::PlannedGain(gain) => {
                if let Some(l) = self.loaded.as_mut() {
                    l.gain = gain;
                }
                self.meters.set_out_offset(gain);
                self.with_feeder(|f| f.set_planned_gain(gain));
            }
            Command::Listen(listen) => {
                if let Some(l) = self.loaded.as_mut() {
                    l.listen = listen;
                }
                self.with_feeder(|f| f.set_listen(listen));
            }
            Command::Volume(volume) => {
                self.volume = volume;
                self.with_feeder(|f| f.set_volume(volume));
            }
        }
    }

    fn play(&mut self, from: Option<u64>) {
        let Some(l) = self.loaded.as_mut() else {
            return;
        };
        if l.open.is_none() {
            match open(l, self.volume, &self.meters) {
                Ok(o) => {
                    l.open = Some(o);
                    self.error = None;
                }
                Err(e) => {
                    self.error = Some(e.to_string());
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

    fn with_feeder(&mut self, f: impl FnOnce(&mut Feeder)) {
        if let Some(open) = self.loaded.as_mut().and_then(|l| l.open.as_mut()) {
            f(&mut open.feeder);
        }
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

/// Opens the default output device for `l` at `volume`, continuing at `l.resume_at` with its
/// readings in `meters`.
fn open(l: &Loaded, volume: Option<DbFs>, meters: &Arc<Meters>) -> Result<Open> {
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
    let mut renderer = Renderer::new(Arc::clone(&l.track), rate, channels)?;
    renderer.set_grid(l.grid.clone());
    renderer.set_click(l.click);
    renderer.seek(l.resume_at);
    let mut feeder = Feeder::new(
        renderer,
        producer,
        Arc::clone(&shared),
        Arc::clone(meters),
        rate,
        channels,
    )?;
    feeder.set_planned_gain(l.gain);
    feeder.set_listen(l.listen);
    feeder.set_volume(volume);
    let mut callback = Callback::new(consumer, Arc::clone(&shared), channels, rate);
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
    Ok(Open {
        feeder,
        failed,
        _stream: stream,
    })
}

#[cfg(test)]
mod tests;
