//! The grid view's command bodies, as plain Rust tested without a window: opening a track
//! (analysing it again when the cache lacks its evidence), waveform bins, the cover, kick onsets,
//! refits, saving an edit, and the click player. `lib.rs` wraps each in a one-line command.
//!
//! One track is open at a time. Its analysis record (evidence included) and its decoded audio
//! stay here until another track opens or the view closes; the webview gets bins, residuals and
//! events, never samples.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use sc_core::analysis::{AnalysisRecord, Grid, GridEdit};
use sc_core::ipc::{GridFitHeader, IpcError, RowUpdate, TrackEvent, TrackOpened};
use sc_core::{Bpm, DbFs, Error, SampleIndex};
use sc_engine::edits::{audio_of, refit_record, snap_onsets};
use sc_engine::player::Player;
use sc_engine::{
    Analyzer, CancelToken, Progress, Timings, Track, TrackProgress, apply_saved, save_edit,
};
use sc_io::cache::Cache;
use sc_io::tags;

use crate::shell::Shell;

/// How often the player's position goes to the view (30 Hz).
const PLAYER_TICK: Duration = Duration::from_millis(33);

/// The track open in the grid view.
pub(crate) struct OpenTrack {
    file_id: u32,
    record: AnalysisRecord,
    track: Arc<Track>,
    bpm_range: (Bpm, Bpm),
    /// Ends the thread that reports the player's position.
    stop: Arc<AtomicBool>,
}

impl Drop for OpenTrack {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Sends one event to the grid view.
type Send = Arc<dyn Fn(TrackEvent) + std::marker::Send + Sync>;

fn not_open(file_id: u32) -> IpcError {
    IpcError {
        file_id: Some(file_id),
        ..IpcError::from(Error::InvalidArgument(format!(
            "track {file_id} is not open in the grid view"
        )))
    }
}

impl Shell {
    fn view(&self) -> MutexGuard<'_, Option<OpenTrack>> {
        self.inner
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn player(&self) -> MutexGuard<'_, Option<Player>> {
        self.inner
            .player
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the row `file_id` in the grid view, sending its events to `send`: the analysis
    /// with its evidence (from the cache, or analysed again first), the saved edit applied,
    /// decoding started and the player loaded at the planned gain with the click on.
    ///
    /// # Errors
    /// `invalidArgument` for an unknown row; the analysis's or the decoder's error otherwise.
    pub fn open_track(
        &self,
        file_id: u32,
        send: impl Fn(TrackEvent) + std::marker::Send + Sync + 'static,
    ) -> Result<TrackOpened, IpcError> {
        let send: Send = Arc::new(send);
        let entry = self
            .session()
            .entry(file_id)
            .ok_or_else(|| not_open(file_id))?;
        let path = PathBuf::from(&entry.path);
        let mut settings = self
            .inner
            .analysis
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        settings.grid = true;
        let record = self.record_with_evidence(&path, &settings, &send)?;

        let mut shown = record.clone();
        let state = self
            .inner
            .edits
            .as_ref()
            .map(|store| apply_saved(&mut shown, store))
            .unwrap_or_default();
        let edit = self
            .inner
            .edits
            .as_ref()
            .and_then(|store| store.get(&record.path))
            .filter(|saved| saved.audio == audio_of(&record))
            .map(|saved| saved.edit)
            .unwrap_or_default();
        let gain = DbFs(self.session().planned_gain(file_id).unwrap_or(0.0));

        let decoding = Arc::clone(&send);
        let track = Arc::new(
            Track::open(&path, record.loudness.sample_peak, move |p| {
                decoding(match p {
                    TrackProgress::Decoded(frames) => TrackEvent::Decoded { frames },
                    TrackProgress::Done(frames) => TrackEvent::Ready { frames },
                    TrackProgress::Failed { error, frames } => TrackEvent::Failed {
                        error: IpcError::from(error),
                        frames,
                    },
                });
            })
            .map_err(IpcError::from)?,
        );
        let cover = tags::cover(&path).map(|c| c.mime.to_owned());
        // Held from loading the player until the track is in place: a refit or save of the
        // track before (which set the player's grid under this lock) cannot land in between.
        let mut view = self.view();
        let load = self.load_player(&track, gain, shown.grid.clone())?;
        let stop = Arc::new(AtomicBool::new(false));
        self.report_player(Arc::clone(&stop), load, send);

        let opened = TrackOpened {
            entry,
            sample_rate: record.spec.sample_rate,
            frames: record.frames,
            gain,
            bpm_range: settings.bpm_range,
            analysed: record.grid.clone(),
            grid: shown.grid,
            edit,
            confirmed: state.confirmed,
            timeline: record.loudness.timeline.clone(),
            cover,
        };
        *view = Some(OpenTrack {
            file_id,
            record,
            track,
            bpm_range: settings.bpm_range,
            stop,
        });
        Ok(opened)
    }

    /// The cached analysis of `path` with its grid evidence, or a new analysis (written to the
    /// cache) when the cache has none or an older one without evidence.
    fn record_with_evidence(
        &self,
        path: &std::path::Path,
        settings: &sc_core::analysis::AnalysisSettings,
        send: &Send,
    ) -> Result<AnalysisRecord, IpcError> {
        let (nfc, key) = Cache::key_for(path, settings)?;
        let cached = self.inner.cache.as_ref().and_then(|c| c.get(&nfc, &key));
        if let Some(record) = cached.filter(|r| r.evidence.is_some() || r.grid.is_none()) {
            return Ok(record);
        }
        let progress_send = Arc::clone(send);
        let progress: Progress = Arc::new(move |fraction| {
            progress_send(TrackEvent::Analysing { fraction });
        });
        let mut analyzer = Analyzer::load(settings.clone(), None, CancelToken::new())?;
        let record = analyzer
            .analyze_with(path, &mut Timings::default(), Some(&progress))?
            .record;
        if let Some(cache) = &self.inner.cache {
            cache.put(&nfc, &key, &record)?;
        }
        Ok(record)
    }

    /// Loads `track` into the click player (started on first use), paused at its start.
    /// Loads `track` into the player (started on first use); returns the load's number.
    fn load_player(
        &self,
        track: &Arc<Track>,
        gain: DbFs,
        grid: Option<Grid>,
    ) -> Result<u64, IpcError> {
        let mut player = self.player();
        let p = match player.as_mut() {
            Some(p) => p,
            None => player.insert(Player::new()?),
        };
        let load = p.load(Arc::clone(track), gain);
        p.set_grid(grid.map(Arc::new));
        p.set_click(true);
        Ok(load)
    }

    /// Sends the player's position at 30 Hz while it plays (and once when it stops), and a
    /// device error once, until `stop` is set. Only the state of load `load` is sent: until the
    /// player has taken the track, its status still describes the track before (maybe playing,
    /// far into it), which is not this track's.
    fn report_player(&self, stop: Arc<AtomicBool>, load: u64, send: Send) {
        let shell = self.clone();
        let spawned = std::thread::Builder::new()
            .name("sc-player-state".into())
            .spawn(move || {
                let (mut was_playing, mut last_error) = (false, None);
                while !stop.load(Ordering::Acquire) {
                    let status = shell.player().as_ref().map(Player::status);
                    if let Some(status) = status.filter(|s| s.load == load) {
                        if let Some(state) = status.state
                            && (state.playing || was_playing)
                        {
                            send(TrackEvent::Player {
                                playing: state.playing,
                                position: state.position,
                                underruns: state.underruns,
                            });
                            was_playing = state.playing;
                        }
                        if status.error.is_some() && status.error != last_error {
                            send(TrackEvent::PlayerError {
                                message: status.error.clone().unwrap_or_default(),
                            });
                        }
                        last_error = status.error;
                    }
                    std::thread::sleep(PLAYER_TICK);
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "cannot report the player's position");
        }
    }

    /// The grid the click follows. Called with the view lock held (lock order: view, then
    /// player), so it applies to the track that is open.
    fn set_player_grid(&self, grid: Option<Grid>) {
        if let Some(p) = self.player().as_ref() {
            p.set_grid(grid.map(Arc::new));
        }
    }

    /// Closes the grid view: the player stops and the track's audio is released.
    pub fn close_track(&self) {
        if let Some(p) = self.player().as_ref() {
            p.unload();
        }
        *self.view() = None;
    }

    /// Runs `f` on the open track when it is `file_id`.
    fn with_open<T>(
        &self,
        file_id: u32,
        f: impl FnOnce(&OpenTrack) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        let view = self.view();
        match view.as_ref() {
            Some(open) if open.file_id == file_id => f(open),
            _ => Err(not_open(file_id)),
        }
    }

    /// Waveform bins of the open track as little-endian `i16` min/max pairs
    /// ([`Track::peaks`]).
    ///
    /// # Errors
    /// `invalidArgument` when the track is not open or the request is outside the limits.
    pub fn peaks(
        &self,
        file_id: u32,
        samples_per_bin: u64,
        first_bin: u64,
        bins: u64,
    ) -> Result<Vec<u8>, IpcError> {
        self.with_open(file_id, |open| {
            let pairs = open.track.peaks(samples_per_bin, first_bin, bins)?;
            Ok(pairs.iter().flat_map(|v| v.to_le_bytes()).collect())
        })
    }

    /// The open track's embedded cover as stored, or no bytes.
    ///
    /// # Errors
    /// `invalidArgument` when the track is not open.
    pub fn cover(&self, file_id: u32) -> Result<Vec<u8>, IpcError> {
        self.with_open(file_id, |open| {
            Ok(tags::cover(std::path::Path::new(&open.record.path))
                .map(|c| c.bytes)
                .unwrap_or_default())
        })
    }

    /// The attacks bar 1 snaps to while it is dragged, as little-endian `f64` seconds: the
    /// kick-band onsets, or the broadband ones when the kick band is nearly silent (the
    /// solver's own choice).
    ///
    /// # Errors
    /// `invalidArgument` when the track is not open.
    pub fn onsets(&self, file_id: u32) -> Result<Vec<u8>, IpcError> {
        self.with_open(file_id, |open| {
            Ok(snap_onsets(&open.record)
                .iter()
                .flat_map(|t| t.to_le_bytes())
                .collect())
        })
    }

    /// The grid `edit` gives on the open track, with its per-line residuals, as the bytes
    /// [`GridFitHeader`] describes. Nothing is saved; the click follows the new grid.
    ///
    /// # Errors
    /// `invalidArgument` when the track is not open or the edit is outside its limits.
    pub fn refit(&self, file_id: u32, edit: &GridEdit) -> Result<Vec<u8>, IpcError> {
        edit.validate()?;
        let (header, residuals) = self.with_open(file_id, |open| {
            let solved = refit_record(&open.record, open.bpm_range, edit);
            // The click follows the edit, set while this track is surely the open one.
            self.set_player_grid(solved.as_ref().map(|s| s.grid.clone()));
            Ok(match solved {
                Some(s) => (
                    GridFitHeader {
                        grid: Some(s.grid),
                        first_line: s.lines.first_line,
                        lines: u32::try_from(s.lines.residuals_ms.len()).unwrap_or(u32::MAX),
                        worst_line: s.lines.worst_line,
                        matched: s.lines.matched,
                        attacks: s.lines.attacks,
                    },
                    s.lines.residuals_ms,
                ),
                None => (
                    GridFitHeader {
                        grid: None,
                        first_line: 0,
                        lines: 0,
                        worst_line: None,
                        matched: 0,
                        attacks: 0,
                    },
                    Vec::new(),
                ),
            })
        })?;
        let json = serde_json::to_vec(&header)
            .map_err(|e| IpcError::from(Error::Internal(e.to_string())))?;
        let mut bytes = Vec::with_capacity(4 + json.len() + 4 * residuals.len());
        bytes.extend_from_slice(&u32::try_from(json.len()).unwrap_or(u32::MAX).to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes.extend(residuals.iter().flat_map(|r| r.to_le_bytes()));
        Ok(bytes)
    }

    /// Saves `edit` of the open track, confirmed or not, and returns its row and plan; the
    /// click follows the saved grid.
    ///
    /// # Errors
    /// `invalidArgument` when the track is not open, the edit is outside its limits or gives no
    /// grid, or edits cannot be saved on this machine; `io` when the edit cannot be written.
    pub fn commit(
        &self,
        file_id: u32,
        edit: &GridEdit,
        confirm: bool,
    ) -> Result<RowUpdate, IpcError> {
        let store = self.inner.edits.as_ref().ok_or_else(|| {
            IpcError::from(Error::InvalidArgument("no place to save grid edits".into()))
        })?;
        let (grid, state) = self.with_open(file_id, |open| {
            let state = save_edit(store, &open.record, open.bpm_range, edit, confirm)?;
            let mut shown = open.record.clone();
            apply_saved(&mut shown, store);
            self.set_player_grid(shown.grid.clone());
            Ok((shown.grid, state))
        })?;
        self.session()
            .set_grid(file_id, grid, state)
            .ok_or_else(|| not_open(file_id))
    }

    /// Plays from `from`, or from where it stopped.
    pub fn play(&self, from: Option<SampleIndex>) {
        if let Some(p) = self.player().as_ref() {
            p.play(from);
        }
    }

    /// Pauses the player.
    pub fn pause(&self) {
        if let Some(p) = self.player().as_ref() {
            p.pause();
        }
    }

    /// Moves the player to `to`.
    pub fn seek(&self, to: SampleIndex) {
        if let Some(p) = self.player().as_ref() {
            p.seek(to);
        }
    }

    /// Turns the click on or off.
    pub fn set_click(&self, on: bool) {
        if let Some(p) = self.player().as_ref() {
            p.set_click(on);
        }
    }
}

#[cfg(test)]
mod tests;
