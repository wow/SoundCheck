//! The SoundCheck desktop shell: thin Tauri commands over the engine. No DSP and no file parsing
//! live here; commands map UI intent to engine calls and stream results back.
#![allow(missing_docs)]
#![forbid(unsafe_code)]
// Tauri hands commands their `State` and `Channel` by value.
#![allow(clippy::needless_pass_by_value)]

mod grid_view;
mod shell;

use sc_core::analysis::GridEdit;
use sc_core::ipc::{
    AnalyzeRequest, FileEntry, IpcError, IpcErrorKind, JobEvent, JobId, Replan, RowUpdate,
    SessionSnapshot, TrackEvent, TrackOpened,
};
use sc_core::plan::DecideSettings;
use sc_core::{Lufs, SampleIndex};
use tauri::State;
use tauri::ipc::{Channel, Response};

use crate::shell::Shell;

/// The application version shown in the About panel.
#[tauri::command]
fn app_version() -> String {
    sc_core::VERSION.to_string()
}

/// Runs `f` on a blocking thread, so the command never holds up the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| IpcError {
            kind: IpcErrorKind::Internal,
            message: format!("the command failed: {e}"),
            file_id: None,
        })?
}

/// Adds the audio files under the dropped or chosen paths; returns the new rows.
#[tauri::command]
async fn expand_paths(
    shell: State<'_, Shell>,
    paths: Vec<String>,
) -> Result<Vec<FileEntry>, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || Ok(shell.expand(paths))).await
}

/// Starts analysing files; events arrive on `on_event`, `finished` last.
#[tauri::command]
async fn analyze(
    shell: State<'_, Shell>,
    req: AnalyzeRequest,
    on_event: Channel<JobEvent>,
) -> Result<JobId, IpcError> {
    shell.start(req, move |event| {
        // A closed channel means the window went away; the job still ends normally.
        let _ = on_event.send(event);
    })
}

/// Asks a running job to stop.
#[tauri::command]
async fn cancel_job(shell: State<'_, Shell>, job_id: JobId) -> Result<bool, IpcError> {
    Ok(shell.cancel(job_id))
}

/// Replans every analysed row under new loudness settings.
#[tauri::command]
async fn set_decide_settings(
    shell: State<'_, Shell>,
    settings: DecideSettings,
) -> Result<Replan, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || shell.set_decide_settings(settings)).await
}

/// The median S-P95 of the analysed rows.
#[tauri::command]
async fn calibration_target(shell: State<'_, Shell>) -> Result<Option<Lufs>, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || Ok(shell.calibration_target())).await
}

/// Empties the track list (running jobs are cancelled); files on disk are not touched.
#[tauri::command]
async fn clear_session(shell: State<'_, Shell>) -> Result<(), IpcError> {
    let shell = shell.inner().clone();
    blocking(move || {
        shell.clear();
        Ok(())
    })
    .await
}

/// Every row the session holds, for a window that reloads; running jobs are cancelled.
#[tauri::command]
async fn restore_session(shell: State<'_, Shell>) -> Result<SessionSnapshot, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || Ok(shell.restore())).await
}

/// Opens a row in the grid view; decoding, analysis and player events arrive on `on_event`.
#[tauri::command]
async fn track_open(
    shell: State<'_, Shell>,
    file_id: u32,
    on_event: Channel<TrackEvent>,
) -> Result<TrackOpened, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || {
        shell.open_track(file_id, move |event| {
            // A closed channel means the view went away; the next open replaces this track.
            let _ = on_event.send(event);
        })
    })
    .await
}

/// Closes the grid view: the player stops and the track's audio is released.
#[tauri::command]
async fn track_close(shell: State<'_, Shell>) -> Result<(), IpcError> {
    let shell = shell.inner().clone();
    blocking(move || {
        shell.close_track();
        Ok(())
    })
    .await
}

/// Waveform bins of the open track: little-endian `i16` min/max pairs.
#[tauri::command]
async fn read_peaks(
    shell: State<'_, Shell>,
    file_id: u32,
    samples_per_bin: u64,
    first_bin: u64,
    bins: u64,
) -> Result<Response, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || shell.peaks(file_id, samples_per_bin, first_bin, bins))
        .await
        .map(Response::new)
}

/// The open track's embedded cover, or no bytes.
#[tauri::command]
async fn track_cover(shell: State<'_, Shell>, file_id: u32) -> Result<Response, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || shell.cover(file_id))
        .await
        .map(Response::new)
}

/// The attacks bar 1 snaps to: little-endian `f64` seconds.
#[tauri::command]
async fn track_onsets(shell: State<'_, Shell>, file_id: u32) -> Result<Response, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || shell.onsets(file_id))
        .await
        .map(Response::new)
}

/// The grid an edit gives, with its residuals (header length, JSON header, `f32` residuals).
#[tauri::command]
async fn grid_refit(
    shell: State<'_, Shell>,
    file_id: u32,
    edit: GridEdit,
) -> Result<Response, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || shell.refit(file_id, &edit))
        .await
        .map(Response::new)
}

/// Saves an edit of the open track, confirmed or not; returns its row and plan.
#[tauri::command]
async fn grid_commit(
    shell: State<'_, Shell>,
    file_id: u32,
    edit: GridEdit,
    confirm: bool,
) -> Result<RowUpdate, IpcError> {
    let shell = shell.inner().clone();
    blocking(move || shell.commit(file_id, &edit, confirm)).await
}

/// Plays the open track from `from`, or from where it stopped.
#[tauri::command]
async fn player_play(shell: State<'_, Shell>, from: Option<SampleIndex>) -> Result<(), IpcError> {
    shell.play(from);
    Ok(())
}

/// Pauses the player.
#[tauri::command]
async fn player_pause(shell: State<'_, Shell>) -> Result<(), IpcError> {
    shell.pause();
    Ok(())
}

/// Moves the player to `to`.
#[tauri::command]
async fn player_seek(shell: State<'_, Shell>, to: SampleIndex) -> Result<(), IpcError> {
    shell.seek(to);
    Ok(())
}

/// Turns the click on or off.
#[tauri::command]
async fn player_set_click(shell: State<'_, Shell>, on: bool) -> Result<(), IpcError> {
    shell.set_click(on);
    Ok(())
}

/// Builds and runs the application.
///
/// # Panics
/// If the Tauri runtime fails to start, which is unrecoverable.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(Shell::for_app())
        .invoke_handler(tauri::generate_handler![
            app_version,
            expand_paths,
            analyze,
            cancel_job,
            set_decide_settings,
            calibration_target,
            restore_session,
            clear_session,
            track_open,
            track_close,
            read_peaks,
            track_cover,
            track_onsets,
            grid_refit,
            grid_commit,
            player_play,
            player_pause,
            player_seek,
            player_set_click
        ])
        .run(tauri::generate_context!())
        .expect("the Tauri runtime failed to start");
}

#[cfg(test)]
mod tests;
