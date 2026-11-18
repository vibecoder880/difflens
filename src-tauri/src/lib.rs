//! DiffLens Tauri shell. Thin layer: commands live in `commands.rs` and
//! delegate all logic to `difflens-core`.

pub mod commands;
pub mod state;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running DiffLens");
}
