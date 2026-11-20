//! DiffLens Tauri shell. Thin layer: commands live in `commands.rs` and
//! delegate all logic to `difflens-core`.

pub mod commands;
pub mod state;

use tauri::Manager;

/// Compiler-rt availability stub required by the ONNX Runtime CoreML
/// objects (they are compiled with newer clang availability checks than
/// the compiler-rt variant rustc links for `aarch64-apple-darwin` min 11.0,
/// and `-nodefaultlibs` prevents clang from adding a newer libclang_rt).
///
/// Mirrors `__isPlatformVersionAtLeast` from LLVM's apple_availability.c:
/// returns 1 when the running OS version satisfies (major, minor, patch).
/// Platform 2 is macOS in the LLVM convention; unknown platforms resolve
/// optimistically (ORT only queries macOS here). Read from
/// SystemVersion.plist via std — no objc/libc dependency, ~30 lines.
///
/// Kept alive: called once from `setup` on macOS (release profile strips
/// unreferenced symbols).
#[cfg(target_os = "macos")]
pub fn macos_version_tuple() -> Option<(u32, u32)> {
    let plist = std::fs::read_to_string(
        "/System/Library/CoreServices/SystemVersion.plist",
    )
    .ok()?;
    let idx = plist.find("<key>ProductVersion</key>")?;
    let rest = &plist[idx..];
    let open = rest.find("<string>")? + "<string>".len();
    let close = rest[open..].find("</string>")? + open;
    let version = rest[open..close].trim();
    let mut parts = version.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor))
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "C" fn __isPlatformVersionAtLeast(
    platform: i32,
    major: i32,
    minor: i32,
    _patch: i32,
) -> i32 {
    // Platform 2 == macOS per the LLVM availability convention; other
    // platforms are not applicable to this binary.
    if platform != 2 {
        return 1;
    }
    let Some((os_major, os_minor)) = macos_version_tuple() else {
        return 1; // cannot determine — optimistic (matches compiler-rt tone)
    };
    // The plist carries two components; patch granularity is ignored.
    let (os_major_i, os_minor_i) = (os_major as i32, os_minor as i32);
    if os_major_i > major {
        return 1;
    }
    if os_major_i == major && os_minor_i >= minor {
        return 1;
    }
    0
}

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
            // Point the core's PDFium loader and the e5 embedder at the
            // bundled app resources. Packaged builds ship
            // `resources/pdfium/<lib>` and
            // `resources/models/e5-small/{onnx,tokenizer}` (see
            // tauri.conf.json `bundle.resources`); dev builds fall through
            // to the staged repo copies. Edition-2024 set_var is unsafe and
            // safe here: single-threaded setup, before any reader spawns.
            if let Ok(resource_dir) = app.path().resource_dir() {
                // SAFETY: single-threaded setup window; all readers spawn later.
                unsafe {
                    std::env::set_var(
                        difflens_core::extract::pdfium_loader::DIFFLENS_RESOURCE_DIR_ENV,
                        &resource_dir,
                    );
                    std::env::set_var(
                        "DIFFLENS_E5_DIR",
                        resource_dir.join("models").join("e5-small"),
                    );
                }
            }
            // Reference the availability stub so release stripping keeps it
            // (it satisfies ort-sys's CoreML objects at link time).
            #[cfg(target_os = "macos")]
            {
                let ok = __isPlatformVersionAtLeast(2, 13, 0, 0);
                tracing::debug!("macOS availability check: {ok}");
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
