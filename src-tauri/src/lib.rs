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
        .manage(state::AppState::default())
        .setup(|app| {
            // Point the core's PDFium loader at the bundled app resources.
            // Packaged builds ship `resources/pdfium/<lib>` (see
            // tauri.conf.json `bundle.resources`); dev builds fall through
            // to the staged repo copy. Edition-2024 set_var is unsafe and
            // safe here: single-threaded setup, before any reader spawns.
            if let Ok(resource_dir) = app.path().resource_dir() {
                // SAFETY: single-threaded setup window; all readers spawn later.
                unsafe {
                    std::env::set_var(
                        difflens_core::extract::pdfium_loader::DIFFLENS_RESOURCE_DIR_ENV,
                        &resource_dir,
                    );
                }
            }
            // Phase-08: open the session history store eagerly so every
            // finished diff persists. A failure disables history without
            // blocking the app (commands re-open lazily as a fallback).
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app data dir: {e}"))?;
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!("cannot create app data dir {dir:?}: {e}");
                return Ok(());
            }
            match difflens_core::store::Store::open(&dir.join("difflens.sqlite3")) {
                Ok(store) => {
                    *app.state::<state::AppState>()
                        .store
                        .lock()
                        .expect("store lock") = Some(store);
                }
                Err(e) => tracing::warn!("session store unavailable, history disabled: {e}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::run_diff,
            commands::get_diff,
            commands::cancel_diff,
            commands::ocr_status,
            commands::llm_status,
            commands::llm_install_server,
            commands::llm_download_model,
            commands::explain_change,
            commands::llm_cancel,
            commands::list_sessions,
            commands::get_session,
            commands::delete_session,
            commands::export_report,
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
