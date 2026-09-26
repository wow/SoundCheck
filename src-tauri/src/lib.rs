//! The SoundCheck desktop shell: thin Tauri commands over the engine. No DSP and no file parsing
//! live here; commands map UI intent to engine calls and stream results back.
#![allow(missing_docs)]
// Tauri hands commands their `State` and `Channel` by value.
#![allow(clippy::needless_pass_by_value)]

mod shell;

use sc_core::Lufs;
use sc_core::ipc::{AnalyzeRequest, FileEntry, IpcError, IpcErrorKind, JobEvent, JobId, RowPlan};
use sc_core::plan::DecideSettings;
use tauri::State;
use tauri::ipc::Channel;

use crate::shell::Shell;

/// The application version shown in the About panel.
#[tauri::command]
fn app_version() -> String {
    sc_core::VERSION.to_string()
}

/// Adds the audio files under the dropped or chosen paths; returns the new rows.
#[tauri::command]
async fn expand_paths(
    shell: State<'_, Shell>,
    paths: Vec<String>,
) -> Result<Vec<FileEntry>, IpcError> {
    let shell = shell.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shell.expand(paths))
        .await
        .map_err(|e| IpcError {
            kind: IpcErrorKind::Internal,
            message: format!("adding files failed: {e}"),
            file_id: None,
        })
}

/// Starts analysing files; events arrive on `on_event`, `finished` last.
#[tauri::command]
fn analyze(shell: State<'_, Shell>, req: AnalyzeRequest, on_event: Channel<JobEvent>) -> JobId {
    shell.start(req, move |event| {
        // A closed channel means the window went away; the job still ends normally.
        let _ = on_event.send(event);
    })
}

/// Asks a running job to stop.
#[tauri::command]
fn cancel_job(shell: State<'_, Shell>, job_id: JobId) -> bool {
    shell.cancel(job_id)
}

/// Replans every analysed row under new loudness settings.
#[tauri::command]
fn set_decide_settings(shell: State<'_, Shell>, settings: DecideSettings) -> Vec<RowPlan> {
    shell.set_decide_settings(settings)
}

/// The median S-P95 of the analysed rows.
#[tauri::command]
fn calibration_target(shell: State<'_, Shell>) -> Option<Lufs> {
    shell.calibration_target()
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
            calibration_target
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
