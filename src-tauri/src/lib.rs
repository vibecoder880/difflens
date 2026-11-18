//! DiffLens Tauri shell. Thin layer: commands live in `commands.rs` and
//! delegate all logic to `difflens-core`.

pub mod commands;
pub mod state;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::run_diff,
            commands::get_diff,
            commands::cancel_diff,
            commands::llm_status,
            commands::llm_install_server,
            commands::llm_download_model,
            commands::explain_change,
            commands::llm_cancel,
        ])
        .build(tauri::generate_context!())
        .expect("error while building DiffLens")
        .run(|app_handle, event| {
            // Deterministic sidecar teardown: kill llama-server when the
            // app exits (the Drop impls are the backstop).
            if let tauri::RunEvent::Exit = event {
                let state = app_handle.state::<state::AppState>();
                state.llm.sidecar.shutdown();
            }
        });
}
