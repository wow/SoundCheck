//! The SoundCheck desktop shell: thin Tauri commands over the engine. No DSP and no file parsing
//! live here; commands map UI intent to engine calls and stream results back.
#![allow(missing_docs)]
// Tauri hands commands their `State` and `Channel` by value.
#![allow(clippy::needless_pass_by_value)]

mod shell;

use sc_core::Lufs;
use sc_core::ipc::{
    AnalyzeRequest, FileEntry, IpcError, IpcErrorKind, JobEvent, JobId, Replan, SessionSnapshot,
};
use sc_core::plan::DecideSettings;
use tauri::State;
use tauri::ipc::Channel;

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
            clear_session
        ])
        .run(tauri::generate_context!())
        .expect("the Tauri runtime failed to start");
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_core() {
        assert_eq!(super::app_version(), sc_core::VERSION);
    }
}
