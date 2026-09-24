//! The SoundCheck desktop shell: thin Tauri commands over the engine. No DSP and no file parsing
//! live here; commands map UI intent to engine calls and stream results back.
#![allow(missing_docs)]

/// The application version shown in the About panel.
#[tauri::command]
fn app_version() -> String {
    sc_core::VERSION.to_string()
}

/// Builds and runs the application.
///
/// # Panics
/// If the Tauri runtime fails to start, which is unrecoverable.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![app_version])
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
