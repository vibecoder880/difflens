//! Tauri commands for the diff pipeline: `run_diff` / `get_diff` /
//! `cancel_diff` plus `diff://progress` event emission.
//!
//! Thin shell: the job body delegates to
//! [`difflens_core::pipeline::run_with_progress`]; this layer only tracks
//! jobs, emits progress and honors cancelation at pipeline stage
//! boundaries.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use difflens_core::model::DiffResult;
use difflens_core::store::{NewSession, SessionSummary, Store, StoredSession};

use crate::state::{AppState, DiffState, JobSnapshot, JobState, StoreHandle};

/// Event name for progress updates. Payload: [`ProgressPayload`]
/// (job id + stage + percent; stages: extract/align/diff/done).
pub const PROGRESS_EVENT: &str = "diff://progress";

/// Payload emitted on `diff://progress`.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    pub job_id: String,
    pub stage: String,
    pub percent: u8,
}

/// Start a diff job. Returns the initial snapshot including the job id.
///
/// Trust model: `old_path`/`new_path` are user-chosen documents picked via
/// the native open dialog (the webview never types free-form paths by
/// design). As defense-in-depth the backend still requires both paths to
/// be non-empty existing regular files, so a compromised webview cannot
/// use this command as an arbitrary-file-read primitive.
///
/// `ocr_enabled`: `None` keeps the default (auto) OCR behavior; `Some(false)`
/// disables the experimental OCR fallback entirely, `Some(true)` is
/// equivalent to the default.
#[tauri::command]
pub fn run_diff(
    old_path: String,
    new_path: String,
    ocr_enabled: Option<bool>,
    app: AppHandle,
    state: State<AppState>,
) -> Result<JobSnapshot, String> {
    validate_input_file(&old_path)?;
    validate_input_file(&new_path)?;
    let diff = state.diff.clone();
    let job_id = diff.next_job_id();
    let cancel = diff.insert_running(&job_id)?;

    let job_id_task = job_id.clone();
    let app_task = app.clone();
    let diff_task = state.diff.clone();
    let store_task = state.store.clone();
    // The pipeline body is fully synchronous and CPU-heavy (extraction,
    // alignment, diff); run it on the blocking pool (review P2-8) so it
    // never occupies an async-runtime worker. Progress emission from the
    // blocking thread is fine: `AppHandle` is Send + Sync and `emit` is
    // thread-safe.
    tauri::async_runtime::spawn_blocking(move || {
        execute_job(
            &job_id_task,
            &old_path,
            &new_path,
            ocr_enabled,
            &diff_task,
            cancel,
            Some(&store_task),
            |stage, percent| {
                let _ = app_task.emit(
                    PROGRESS_EVENT,
                    ProgressPayload {
                        job_id: job_id_task.clone(),
                        stage: stage.to_string(),
                        percent,
                    },
                );
            },
        );
    });

    diff.snapshot(&job_id)?
        .ok_or_else(|| "job record vanished".to_string())
}

/// Defense-in-depth for webview-supplied document paths: require a
/// non-empty path pointing at an existing regular file (see `run_diff`).
fn validate_input_file(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("document path is empty".into());
    }
    if !PathBuf::from(path).is_file() {
        return Err(format!("document path is not a readable file: {path}"));
    }
    Ok(())
}

/// Fetch the current snapshot (status + result) of a diff job.
///
/// Ids in the `session-<store id>` namespace are hydrated from the
/// persisted history (phase-08): the returned snapshot is `done` with the
/// stored `DiffResult`, so the existing `/diff/:jobId` view works
/// unchanged for past sessions.
#[tauri::command]
pub fn get_diff(
    job_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<Option<JobSnapshot>, String> {
    if let Some(snapshot) = state.diff.snapshot(&job_id)? {
        return Ok(Some(snapshot));
    }
    if let Some(store_id) = job_id.strip_prefix("session-") {
        return session_snapshot(&app, store_id, &job_id);
    }
    Ok(None)
}

/// Look up one stored session through the app state store.
fn lookup_stored(app: &AppHandle, store_id: &str) -> Result<Option<StoredSession>, String> {
    let cell = app.state::<AppState>().store.clone();
    let guard = cell.lock().map_err(|e| e.to_string())?;
    match guard.as_ref() {
        Some(db) => db.get_session(store_id).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Build a done snapshot from a stored session row (`result` is `None` for
/// corrupt rows — the UI then shows its not-found state).
fn session_snapshot(
    app: &AppHandle,
    store_id: &str,
    route_id: &str,
) -> Result<Option<JobSnapshot>, String> {
    Ok(lookup_stored(app, store_id)?.map(|session| JobSnapshot {
        job_id: route_id.to_string(),
        state: JobState::Done,
        stage: "done".into(),
        percent: 100,
        error: None,
        result: session.result,
    }))
}

// ---------------------------------------------------------------------------
// Phase-08: session history persistence + report export.
// ---------------------------------------------------------------------------

/// Current wall-clock time in epoch milliseconds (shell layer only — core
/// receives timestamps explicitly so it stays deterministic).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Persist a finished diff result (fire-and-forget): failures log a
/// warning and never fail the job.
fn persist_session(
    store: &StoreHandle,
    job_id: &str,
    old_path: &str,
    new_path: &str,
    result: &DiffResult,
) {
    let guard = match store.lock() {
        Ok(guard) => guard,
        Err(e) => {
            tracing::warn!("session store lock poisoned; skip persist {job_id}: {e}");
            return;
        }
    };
    let Some(db) = guard.as_ref() else {
        // Store never opened (setup failed) — history disabled.
        return;
    };
    let record = NewSession {
        id: job_id.to_string(),
        created_at: now_ms(),
        old_path: old_path.to_string(),
        new_path: new_path.to_string(),
        format: result.format,
        stats: result.stats.clone(),
        embeddings_used: result.embeddings_used,
        ocr_used: result.ocr_used,
        result_json: match serde_json::to_string(result) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("session {job_id}: serialize result failed: {e}");
                return;
            }
        },
    };
    if let Err(e) = db.insert_session(&record) {
        tracing::warn!("session persistence failed for {job_id}: {e}");
    }
}

/// Run `f` with the session store, opening it on first use (the app setup
/// opens it eagerly; this is the fallback for headless/tool paths).
fn with_store<T>(
    app: &AppHandle,
    f: impl FnOnce(&Store) -> Result<T, String>,
) -> Result<T, String> {
    let cell = app.state::<AppState>().store.clone();
    let mut guard = cell.lock().map_err(|e| e.to_string())?;
    if guard.is_none() {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
        *guard = Some(Store::open(&dir.join("difflens.sqlite3")).map_err(|e| e.to_string())?);
    }
    f(guard.as_ref().expect("store initialized above"))
}

/// Paged session summaries for the history list (newest first).
#[tauri::command]
pub fn list_sessions(
    limit: u32,
    offset: u32,
    app: AppHandle,
) -> Result<Vec<SessionSummary>, String> {
    with_store(&app, |db| {
        db.list_sessions(limit, offset).map_err(|e| e.to_string())
    })
}

/// Full stored session (with deserialized result); `None` for unknown ids.
#[tauri::command]
pub fn get_session(session_id: String, app: AppHandle) -> Result<Option<StoredSession>, String> {
    with_store(&app, |db| {
        db.get_session(&session_id).map_err(|e| e.to_string())
    })
}

/// Delete one stored session; `true` when a row was removed.
#[tauri::command]
pub fn delete_session(session_id: String, app: AppHandle) -> Result<bool, String> {
    with_store(&app, |db| {
        db.delete_session(&session_id).map_err(|e| e.to_string())
    })
}

/// Render a report for a live job (`job-*`, in memory) or a persisted
/// session (`session-*`, store) and write it to `target_path`. Returns
/// bytes written.
///
/// Trust model: `target_path` is a user-chosen destination from the native
/// save dialog (the frontend only forwards dialog results). As
/// defense-in-depth the backend validates it — non-empty, absolute, not a
/// directory, parent exists, extension matching the format — so a
/// compromised webview does not gain an arbitrary-file-write primitive.
#[tauri::command]
pub fn export_report(
    id: String,
    format: String,
    target_path: String,
    lang: Option<String>,
    app: AppHandle,
    state: State<AppState>,
) -> Result<u64, String> {
    // Resolve (result, created_at): live job first, then stored session.
    let (result, created_at) = if let Some(snapshot) = state.diff.snapshot(&id)? {
        let result = snapshot
            .result
            .ok_or_else(|| format!("diff job {id} has no result yet"))?;
        (result, now_ms())
    } else if let Some(store_id) = id.strip_prefix("session-") {
        let session = lookup_stored(&app, store_id)?
            .ok_or_else(|| format!("unknown session {store_id:?}"))?;
        let created_at = session.created_at;
        let result = session
            .result
            .ok_or_else(|| format!("stored result for {store_id} is corrupt"))?;
        (result, created_at)
    } else {
        return Err(format!("unknown job or session {id:?}"));
    };

    let lang = difflens_core::report::ReportLang::from_tag(lang.as_deref().unwrap_or("en"));
    let version = app.package_info().version.to_string();
    write_report(
        &result,
        &id,
        created_at,
        lang,
        &version,
        &format,
        &target_path,
    )
}

/// Canonical file extension for a report format (`"html"` / `"md"`).
fn report_extension(format: &str) -> Result<&'static str, String> {
    match format {
        "html" => Ok("html"),
        "md" | "markdown" => Ok("md"),
        other => Err(format!(
            "unknown report format {other:?} (expected \"html\" or \"md\")"
        )),
    }
}

/// Defense-in-depth for the webview-supplied report target (see
/// `export_report`): require a non-empty absolute path that is not a
/// directory, whose parent exists and whose extension matches the format
/// (the save dialog enforces all of this for user-chosen paths; a
/// compromised webview must not pick arbitrary file types).
fn validate_report_target(target_path: &str, format: &str) -> Result<PathBuf, String> {
    if target_path.trim().is_empty() {
        return Err("report target path is empty".into());
    }
    let target = PathBuf::from(target_path);
    if !target.is_absolute() {
        return Err(format!(
            "report target path must be absolute: {target_path}"
        ));
    }
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let wanted = report_extension(format)?;
    if ext.as_deref() != Some(wanted) {
        return Err(format!(
            "report file extension must be .{wanted} for format {format:?}: {target_path}"
        ));
    }
    if target.is_dir() {
        return Err(format!("target path is a directory: {target_path}"));
    }
    match target.parent() {
        Some(parent) if !parent.is_dir() => Err(format!(
            "target directory does not exist: {}",
            parent.display()
        )),
        _ => Ok(target),
    }
}

/// Format + validate + write the report file (no Tauri types — headless
/// testable). Returns bytes written.
fn write_report(
    result: &DiffResult,
    id: &str,
    created_at: i64,
    lang: difflens_core::report::ReportLang,
    version: &str,
    format: &str,
    target_path: &str,
) -> Result<u64, String> {
    let input = difflens_core::report::ReportInput {
        id,
        result,
        created_at,
        lang,
        version,
    };
    let content = match format {
        "html" => difflens_core::report::html::render_html(&input),
        "md" | "markdown" => difflens_core::report::markdown::render_markdown(&input),
        other => {
            return Err(format!(
                "unknown report format {other:?} (expected \"html\" or \"md\")"
            ));
        }
    };
    let target = validate_report_target(target_path, format)?;
    let bytes = content.into_bytes();
    std::fs::write(&target, &bytes).map_err(|e| format!("write report: {e}"))?;
    Ok(bytes.len() as u64)
}

/// Request cancelation. `true` when a running job was found and flagged;
/// the pipeline checks the flag at stage boundaries.
#[tauri::command]
pub fn cancel_diff(job_id: String, state: State<AppState>) -> Result<bool, String> {
    state.diff.request_cancel(&job_id)
}

/// Status snapshot for `ocr_status` (phase 06 settings UI guidance).
#[derive(Debug, Serialize)]
pub struct OcrStatus {
    /// True when tesseract + the Vietnamese traineddata are both usable.
    pub available: bool,
    /// Resolved tesseract binary path, when found.
    pub binary: Option<String>,
    /// Languages reported by `tesseract --list-langs`.
    pub langs: Vec<String>,
    /// Vietnamese traineddata present (required for the OCR path).
    pub has_vie: bool,
}

/// Report whether the experimental OCR path can run on this machine.
/// Never fails — detection degrades to `available: false`.
#[tauri::command]
pub fn ocr_status() -> Result<OcrStatus, String> {
    let avail = difflens_core::ocr::detect();
    Ok(OcrStatus {
        available: avail.usable(),
        binary: avail.binary.map(|b| b.to_string_lossy().into_owned()),
        langs: avail.langs,
        has_vie: avail.has_vie,
    })
}

/// The synchronous job body — runs on the async runtime and is testable
/// headless (no `AppHandle` needed; events go through the `emit` closure).
/// `store` (phase-08) receives the finished result for history persistence
/// (fire-and-forget; `None` disables persistence, e.g. in tests).
fn execute_job(
    job_id: &str,
    old_path: &str,
    new_path: &str,
    ocr_enabled: Option<bool>,
    diff: &DiffState,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    store: Option<&StoreHandle>,
    mut emit: impl FnMut(&'static str, u8),
) {
    let cancelled = || cancel.load(Ordering::SeqCst);
    // Phase-06: experimental OCR fallback, auto-enabled by default (runs only
    // when a scanned PDF is detected AND tesseract + vie are installed). The
    // frontend preference can disable it entirely (`Some(false)`).
    let mut ocr = difflens_core::ocr::OcrOptions::default();
    if let Some(enabled) = ocr_enabled {
        ocr.enabled = enabled;
    }
    let result = difflens_core::pipeline::run_with_progress(
        old_path,
        new_path,
        difflens_core::pipeline::default_embedder().as_ref(),
        &ocr,
        &mut |stage, percent| {
            if cancelled() {
                return Err(difflens_core::CoreError::Diff("cancelled by user".into()));
            }
            diff.set_progress(job_id, stage.as_str(), percent);
            emit(stage.as_str(), percent);
            Ok(())
        },
    );

    match result {
        Ok(result) => {
            if let Some(store) = store {
                persist_session(store, job_id, old_path, new_path, &result);
            }
            diff.finish(job_id, result);
        }
        Err(e) => {
            if cancelled() {
                diff.mark_cancelled(job_id);
            } else {
                diff.fail(job_id, e.to_string());
            }
        }
    }
    // Terminal progress event so listeners see the final state transition.
    // Poison (map unreadable) skips the terminal emit instead of panicking
    // inside the job thread (P2-9).
    let snapshot = diff.snapshot(job_id).unwrap_or_else(|e| {
        tracing::warn!("job map lock poisoned; terminal emit for {job_id} skipped: {e}");
        None
    });
    if let Some(snap) = snapshot {
        emit(
            match snap.state {
                JobState::Done => "done",
                JobState::Failed => "failed",
                JobState::Cancelled => "cancelled",
                JobState::Running => "running",
            },
            snap.percent,
        );
    }
}

// ---------------------------------------------------------------------------
// Phase-05: LLM sidecar commands (`llm://download`, `llm://token`,
// `llm://done` events). Server/model paths live under the Tauri app data
// dir; core stays OS-agnostic (explicit paths everywhere).
// ---------------------------------------------------------------------------

/// Progress events for server/model downloads (phase-05).
pub const LLM_DOWNLOAD_EVENT: &str = "llm://download";
/// Streaming token events for explanations.
pub const LLM_TOKEN_EVENT: &str = "llm://token";
/// Terminal event for an explanation job.
pub const LLM_DONE_EVENT: &str = "llm://done";

/// Payload emitted on `llm://download` (server install + model download).
#[derive(Debug, Clone, Serialize)]
pub struct LlmDownloadPayload {
    pub job_id: String,
    /// `server` or `model`.
    pub kind: String,
    /// Model id (empty for server installs).
    pub model_id: String,
    pub state: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub percent: u8,
    pub error: Option<String>,
}

/// Payload emitted per token delta on `llm://token`.
#[derive(Debug, Clone, Serialize)]
pub struct LlmTokenPayload {
    pub job_id: String,
    pub delta: String,
}

/// Payload emitted on `llm://done` (terminal; parsed from model output).
#[derive(Debug, Clone, Serialize)]
pub struct LlmDonePayload {
    pub job_id: String,
    pub ok: bool,
    pub cancelled: bool,
    pub summary: Option<String>,
    pub impact: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: u128,
}

/// Status snapshot for `llm_status`.
#[derive(Debug, Serialize)]
pub struct LlmStatus {
    pub server_tag: String,
    /// Manifest-pinned server installed under app data.
    pub server_installed: bool,
    /// A llama-server found on PATH (fallback).
    pub server_on_path: bool,
    /// Server currently running (healthy loopback sidecar).
    pub server_running: bool,
    pub models: Vec<LlmModelStatus>,
}

#[derive(Debug, Serialize)]
pub struct LlmModelStatus {
    pub id: String,
    pub role: String,
    pub present: bool,
    pub bytes: u64,
    pub downloaded_bytes: u64,
    pub license: String,
    pub verified: bool,
}

fn app_paths(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    Ok((data.join("bin"), data.join("models")))
}

#[tauri::command]
pub fn llm_status(app: AppHandle) -> Result<LlmStatus, String> {
    let manifest = difflens_core::download::manifest::LlmManifest::embedded();
    let (bin_root, models_dir) = app_paths(&app)?;
    let server_installed =
        difflens_core::llm::sidecar::detect_installed(&bin_root, &manifest.llama_server.tag)
            .is_some();
    let models = manifest
        .models
        .iter()
        .map(|m| {
            let path = models_dir.join(&m.file);
            let present = path.is_file();
            let downloaded_bytes = path.metadata().map(|md| md.len()).unwrap_or(0);
            LlmModelStatus {
                id: m.id.clone(),
                role: match m.role {
                    difflens_core::download::manifest::ModelRole::Primary => "primary",
                    difflens_core::download::manifest::ModelRole::Fallback => "fallback",
                    difflens_core::download::manifest::ModelRole::Test => "test",
                }
                .to_string(),
                present,
                bytes: m.bytes,
                downloaded_bytes,
                license: m.license.clone(),
                verified: m.verified,
            }
        })
        .collect();
    Ok(LlmStatus {
        server_tag: manifest.llama_server.tag.clone(),
        server_installed,
        server_on_path: difflens_core::llm::sidecar::detect_on_path().is_some(),
        server_running: app
            .state::<AppState>()
            .llm
            .sidecar
            .active_info()
            .map(|(_, _, healthy)| healthy)
            .unwrap_or(false),
        models,
    })
}

/// Percent from bytes done/total (0 while total is unknown).
fn download_percent(done: u64, total: Option<u64>) -> u8 {
    total
        .map(|t| ((done as f64 / t.max(1) as f64) * 100.0).min(100.0) as u8)
        .unwrap_or(0)
}

/// Running-state `llm://download` payload (shared by the server/model
/// download threads).
fn running_payload(
    job_id: &str,
    kind: &str,
    model_id: &str,
    done: u64,
    total: Option<u64>,
) -> LlmDownloadPayload {
    LlmDownloadPayload {
        job_id: job_id.to_string(),
        kind: kind.to_string(),
        model_id: model_id.to_string(),
        state: "running".into(),
        bytes_done: done,
        bytes_total: total,
        percent: download_percent(done, total),
        error: None,
    }
}

/// Terminal (event state, manager `JobState`, error message) for a
/// download outcome.
fn download_outcome(
    result: &Result<std::path::PathBuf, difflens_core::CoreError>,
) -> (
    &'static str,
    difflens_core::download::JobState,
    Option<String>,
) {
    match result {
        Ok(_) => ("done", difflens_core::download::JobState::Done, None),
        Err(difflens_core::CoreError::Cancelled) => (
            "cancelled",
            difflens_core::download::JobState::Cancelled,
            Some("cancelled by user".into()),
        ),
        Err(e) => (
            "failed",
            difflens_core::download::JobState::Failed,
            Some(e.to_string()),
        ),
    }
}

/// Download + install the manifest-pinned llama-server bundle under
/// `app_data/bin/llama-server-<tag>/`. Progress on `llm://download`;
/// cancelable via `llm_cancel` (the job is registered with the core
/// `DownloadManager`, which owns the cancel flag).
#[tauri::command]
pub fn llm_install_server(app: AppHandle) -> Result<String, String> {
    let manifest = difflens_core::download::manifest::LlmManifest::embedded();
    let asset = manifest
        .server_asset_for_current_os()
        .ok_or_else(|| {
            format!(
                "no pinned llama-server asset for this platform ({})",
                difflens_core::download::manifest::current_target_triple()
            )
        })?
        .clone();
    let tag = manifest.llama_server.tag.clone();
    let (bin_root, _) = app_paths(&app)?;
    let archive_path = bin_root.join(format!("llama-{tag}-{}.archive", asset.id));

    let mut spec = difflens_core::download::DownloadSpec::new(
        asset.url.clone(),
        archive_path.clone(),
        asset.sha256.clone(),
    );
    spec.expected_bytes = Some(asset.bytes);

    let app_task = app.clone();
    let job_id = format!("server-{tag}");
    // Register before spawning so any `llm_cancel(job_id)` (possible only
    // after this command returns the id) finds the job. The progress
    // closure returns `false` when the flag is set, aborting the transfer
    // cooperatively (core keeps the `.part` for a later resume).
    let downloads = app.state::<AppState>().llm.downloads.clone();
    let cancel = downloads.register(&job_id).map_err(|e| e.to_string())?;
    let job_id_thread = job_id.clone();
    let downloads_task = downloads.clone();
    std::thread::Builder::new()
        .name("difflens-install-server".into())
        .spawn(move || {
            let emit = |p: LlmDownloadPayload| {
                let _ = app_task.emit(LLM_DOWNLOAD_EVENT, &p);
            };
            let result = difflens_core::download::download_file(&spec, &mut |done, total| {
                emit(running_payload(&job_id_thread, "server", "", done, total));
                !cancel.load(Ordering::SeqCst)
            })
            .and_then(|archive| {
                difflens_core::llm::sidecar::install_bundle(
                    &archive,
                    asset.archive,
                    &tag,
                    &bin_root,
                    &difflens_core::download::archive::ExtractLimits::default(),
                )
            });

            if result.is_ok() {
                // Reclaim disk: the verified archive is redundant.
                let _ = std::fs::remove_file(&archive_path);
            }
            let (state, job_state, error) = download_outcome(&result);
            downloads_task.finish(&job_id_thread, job_state, error.clone());
            emit(LlmDownloadPayload {
                job_id: job_id_thread,
                kind: "server".into(),
                model_id: String::new(),
                state: state.into(),
                bytes_done: 0,
                bytes_total: Some(asset.bytes),
                percent: if state == "done" { 100 } else { 0 },
                error,
            });
        })
        .map_err(|e| format!("spawn install thread: {e}"))?;
    Ok(job_id)
}

/// Download a manifest-pinned model into `app_data/models/`. Progress on
/// `llm://download`; cancelable via `llm_cancel` (same `DownloadManager`
/// registration as `llm_install_server`).
#[tauri::command]
pub fn llm_download_model(model_id: String, app: AppHandle) -> Result<String, String> {
    let manifest = difflens_core::download::manifest::LlmManifest::embedded();
    let model = manifest
        .model(&model_id)
        .cloned()
        .ok_or_else(|| format!("unknown model id {model_id:?}"))?;
    let (_, models_dir) = app_paths(&app)?;
    let dest = models_dir.join(&model.file);

    let mut spec =
        difflens_core::download::DownloadSpec::new(model.url.clone(), dest, model.sha256.clone());
    spec.expected_bytes = Some(model.bytes);

    let app_task = app.clone();
    let job_id = format!("model-{}", model.id);
    let downloads = app.state::<AppState>().llm.downloads.clone();
    let cancel = downloads.register(&job_id).map_err(|e| e.to_string())?;
    let job_id_thread = job_id.clone();
    let downloads_task = downloads.clone();
    let model_id = model.id.clone();
    std::thread::Builder::new()
        .name(format!("difflens-model-{model_id}"))
        .spawn(move || {
            let emit = |p: LlmDownloadPayload| {
                let _ = app_task.emit(LLM_DOWNLOAD_EVENT, &p);
            };
            let result = difflens_core::download::download_file(&spec, &mut |done, total| {
                emit(running_payload(
                    &job_id_thread,
                    "model",
                    &model_id,
                    done,
                    total,
                ));
                !cancel.load(Ordering::SeqCst)
            });
            let (state, job_state, error) = download_outcome(&result);
            downloads_task.finish(&job_id_thread, job_state, error.clone());
            emit(LlmDownloadPayload {
                job_id: job_id_thread,
                kind: "model".into(),
                model_id,
                state: state.into(),
                bytes_done: if state == "done" { model.bytes } else { 0 },
                bytes_total: Some(model.bytes),
                percent: if state == "done" { 100 } else { 0 },
                error,
            });
        })
        .map_err(|e| format!("spawn download thread: {e}"))?;
    Ok(job_id)
}

/// Explain ONE change of a finished diff job, streaming tokens on
/// `llm://token` and the parsed result on `llm://done`.
#[tauri::command]
pub fn explain_change(
    diff_job_id: String,
    change_id: String,
    lang: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<String, String> {
    use difflens_core::llm::explain::{
        ContextBudget, Lang, build_explanation_messages, change_context_from, parse_explanation,
    };

    // Resolve the change from the stored diff result: live job first, then
    // persisted-session namespace (`session-<store id>`, phase-08).
    let result = if let Some(snapshot) = state.diff.snapshot(&diff_job_id)? {
        snapshot.result.ok_or_else(|| {
            format!(
                "diff job {diff_job_id:?} has no result yet (state {:?})",
                snapshot.state
            )
        })?
    } else if let Some(store_id) = diff_job_id.strip_prefix("session-") {
        let stored = lookup_stored(&app, store_id)?
            .ok_or_else(|| format!("unknown session {store_id:?}"))?;
        stored
            .result
            .ok_or_else(|| format!("stored result for {store_id} is corrupt"))?
    } else {
        return Err(format!("unknown diff job {diff_job_id:?}"));
    };
    let change = result
        .changes
        .iter()
        .find(|c| c.id == change_id)
        .ok_or_else(|| format!("change {change_id:?} not found in job {diff_job_id:?}"))?;
    let ctx = change_context_from(change);

    // Pick the first present manifest model (primary → fallback → test).
    let manifest = difflens_core::download::manifest::LlmManifest::embedded();
    let (_, models_dir) = app_paths(&app)?;
    // Clone the entry so the spawned thread is 'static (no borrow of the
    // local manifest crosses the thread boundary).
    let (model_entry, model_path) = manifest
        .models
        .iter()
        .map(|m| (m.clone(), models_dir.join(&m.file)))
        .find(|(_, p)| p.is_file())
        .ok_or_else(|| "no model downloaded yet; call llm_download_model first".to_string())?;

    // Resolve the server binary: pinned install first, then PATH.
    let (bin_root, _) = app_paths(&app)?;
    let server_bin =
        difflens_core::llm::sidecar::detect_installed(&bin_root, &manifest.llama_server.tag)
            .or_else(difflens_core::llm::sidecar::detect_on_path)
            .ok_or_else(|| {
                "llama-server not installed; call llm_install_server first".to_string()
            })?;

    // Ensure a healthy sidecar (spawns or reuses).
    let mut cfg = difflens_core::llm::sidecar::SidecarConfig::new(server_bin, model_path);
    cfg.context_size = 2048;
    let sidecar = state
        .llm
        .sidecar
        .ensure_running(&cfg)
        .map_err(|e| e.to_string())?;
    let client = sidecar.client().map_err(|e| e.to_string())?;

    let (job_id, cancel) = state.llm.explain.insert_running()?;
    let lang = Lang::from_tag(&lang);
    let messages = build_explanation_messages(&ctx, lang, &ContextBudget::default());

    let app_task = app.clone();
    let explain = state.llm.explain.clone();
    let job_id_task = job_id.clone();
    let model_id = model_entry.id.clone();
    std::thread::Builder::new()
        .name(format!("difflens-explain-{change_id}"))
        .spawn(move || {
            let started = std::time::Instant::now();
            let opts = difflens_core::llm::client::ChatOptions {
                model: model_id,
                max_tokens: 512,
                ..Default::default()
            };
            let emit_token = |delta: &str| {
                let _ = app_task.emit(
                    LLM_TOKEN_EVENT,
                    LlmTokenPayload {
                        job_id: job_id_task.clone(),
                        delta: delta.to_string(),
                    },
                );
            };
            let cancelled = || cancel.load(Ordering::SeqCst);
            let outcome = client.chat_stream(&messages, &opts, &mut |delta| {
                emit_token(delta);
                !cancelled()
            });
            let payload = match outcome {
                Ok(stream) => {
                    let parsed = parse_explanation(&stream.text);
                    LlmDonePayload {
                        job_id: job_id_task.clone(),
                        ok: true,
                        cancelled: false,
                        summary: Some(parsed.summary),
                        impact: Some(parsed.impact),
                        error: None,
                        elapsed_ms: started.elapsed().as_millis(),
                    }
                }
                Err(difflens_core::CoreError::Cancelled) => LlmDonePayload {
                    job_id: job_id_task.clone(),
                    ok: false,
                    cancelled: true,
                    summary: None,
                    impact: None,
                    error: Some("cancelled by user".into()),
                    elapsed_ms: started.elapsed().as_millis(),
                },
                Err(e) => LlmDonePayload {
                    job_id: job_id_task.clone(),
                    ok: false,
                    cancelled: false,
                    summary: None,
                    impact: None,
                    error: Some(e.to_string()),
                    elapsed_ms: started.elapsed().as_millis(),
                },
            };
            let _ = app_task.emit(LLM_DONE_EVENT, &payload);
            explain.remove(&job_id_task);
        })
        .map_err(|e| format!("spawn explain thread: {e}"))?;
    Ok(job_id)
}

/// Cancel an LLM job: a running explain job (`explain-*` ids) or a
/// download (`server-*` / `model-*` ids, registered with the core
/// `DownloadManager`). `true` when a running job was found and flagged.
#[tauri::command]
pub fn llm_cancel(job_id: String, state: State<AppState>) -> Result<bool, String> {
    if job_id.starts_with("explain-") {
        return state.llm.explain.request_cancel(&job_id);
    }
    state
        .llm
        .downloads
        .cancel(&job_id)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DiffState;
    use std::sync::atomic::AtomicBool;

    fn write_temp(name: &str, content: &str) -> String {
        let path = std::env::temp_dir().join(format!("difflens-cmd-test-{name}"));
        std::fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn headless_round_trip_runs_stages_and_finishes() {
        let diff = DiffState::default();
        let job_id = diff.next_job_id();
        let cancel = diff.insert_running(&job_id).unwrap();
        let a = write_temp("a.txt", "# Contract\n\nPayment within 30 days.\n");
        let b = write_temp("b.txt", "# Contract\n\nPayment within 45 days.\n");

        let mut events: Vec<(String, u8)> = Vec::new();
        execute_job(
            &job_id,
            &a,
            &b,
            None,
            &diff,
            cancel,
            None,
            |stage, percent| {
                events.push((stage.to_string(), percent));
            },
        );

        let snap = diff.snapshot(&job_id).unwrap().expect("job present");
        assert_eq!(snap.state, JobState::Done);
        let result = snap.result.expect("result stored");
        assert_eq!(result.format, difflens_core::DocFormat::Txt);
        assert_eq!(result.stats.modified, 1);

        // Every stage was announced, in order, plus the terminal event.
        let stages: Vec<&str> = events.iter().map(|(s, _)| s.as_str()).collect();
        let extract_idx = stages.iter().position(|s| *s == "extract").unwrap();
        let align_idx = stages.iter().position(|s| *s == "align").unwrap();
        let diff_idx = stages.iter().position(|s| *s == "diff").unwrap();
        assert!(extract_idx < align_idx && align_idx < diff_idx);
        assert_eq!(*stages.last().unwrap(), "done");
    }

    #[test]
    fn cancel_before_start_marks_job_cancelled() {
        let diff = DiffState::default();
        let job_id = diff.next_job_id();
        let cancel = diff.insert_running(&job_id).unwrap();
        cancel.store(true, Ordering::SeqCst);
        let a = write_temp("c.txt", "one\n");
        let b = write_temp("d.txt", "two\n");

        execute_job(&job_id, &a, &b, None, &diff, cancel, None, |_, _| {});
        let snap = diff.snapshot(&job_id).unwrap().unwrap();
        assert_eq!(snap.state, JobState::Cancelled);
    }

    #[test]
    fn failing_input_marks_job_failed() {
        let diff = DiffState::default();
        let job_id = diff.next_job_id();
        let cancel = diff.insert_running(&job_id).unwrap();
        execute_job(
            &job_id,
            "/nonexistent/a.txt",
            "/nonexistent/b.txt",
            None,
            &diff,
            cancel,
            None,
            |_, _| {},
        );
        let snap = diff.snapshot(&job_id).unwrap().unwrap();
        assert_eq!(snap.state, JobState::Failed);
        assert!(
            snap.error
                .as_deref()
                .unwrap_or_default()
                .contains("cannot read")
        );
    }

    #[test]
    fn finished_job_persists_session_with_result() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(std::sync::Mutex::new(Some(
            difflens_core::store::Store::open(&dir.path().join("h.sqlite3")).unwrap(),
        )));
        let diff = DiffState::default();
        let job_id = diff.next_job_id();
        let cancel = diff.insert_running(&job_id).unwrap();
        let a = write_temp("persist-a.txt", "# Contract\n\nPayment within 30 days.\n");
        let b = write_temp("persist-b.txt", "# Contract\n\nPayment within 45 days.\n");

        execute_job(
            &job_id,
            &a,
            &b,
            None,
            &diff,
            cancel,
            Some(&store),
            |_, _| {},
        );
        assert_eq!(
            diff.snapshot(&job_id).unwrap().unwrap().state,
            JobState::Done
        );

        let guard = store.lock().unwrap();
        let db = guard.as_ref().unwrap();
        let session = db.get_session(&job_id).unwrap().expect("session stored");
        assert_eq!(session.id, job_id);
        assert!(session.created_at > 0);
        assert_eq!(session.old_path, a);
        assert_eq!(session.new_path, b);
        let result = session.result.expect("result deserializes");
        assert_eq!(result.stats.modified, 1);
        assert!(!result.ocr_used);
        // Listed newest-first with a summary row.
        let listed = db.list_sessions(10, 0).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, job_id);
    }

    #[test]
    fn unopened_store_never_fails_the_job() {
        // History disabled (store not opened): persist is skipped silently
        // and the job still completes.
        let store: StoreHandle = Arc::new(std::sync::Mutex::new(None));
        let diff = DiffState::default();
        let job_id = diff.next_job_id();
        let cancel = diff.insert_running(&job_id).unwrap();
        let a = write_temp("nofail-a.txt", "one\n");
        let b = write_temp("nofail-b.txt", "two\n");

        execute_job(
            &job_id,
            &a,
            &b,
            None,
            &diff,
            cancel,
            Some(&store),
            |_, _| {},
        );
        assert_eq!(
            diff.snapshot(&job_id).unwrap().unwrap().state,
            JobState::Done
        );
    }

    #[test]
    fn write_report_renders_html_and_md_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_temp("report-a.txt", "# Contract\n\nPayment within 30 days.\n");
        let b = write_temp("report-b.txt", "# Contract\n\nPayment within 45 days.\n");
        let diff =
            difflens_core::pipeline::run(&a, &b, &*difflens_core::pipeline::default_embedder())
                .unwrap();

        let html_path = dir.path().join("report.html");
        let written = write_report(
            &diff,
            "job-x",
            1_700_000_000_000,
            difflens_core::report::ReportLang::En,
            "1.0.0",
            "html",
            html_path.to_str().unwrap(),
        )
        .unwrap();
        let html = std::fs::read_to_string(&html_path).unwrap();
        assert_eq!(written, html.len() as u64);
        assert!(html.contains("Payment within"));
        assert!(html.contains("DiffLens 1.0.0"));
        assert!(!html.contains("<script"));

        let md_path = dir.path().join("report.md");
        write_report(
            &diff,
            "job-x",
            1_700_000_000_000,
            difflens_core::report::ReportLang::En,
            "1.0.0",
            "md",
            md_path.to_str().unwrap(),
        )
        .unwrap();
        let md = std::fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("# Document comparison report"));

        // Validation: unknown format and missing parent dir both error.
        assert!(
            write_report(
                &diff,
                "job-x",
                0,
                difflens_core::report::ReportLang::En,
                "1.0.0",
                "pdf",
                html_path.to_str().unwrap()
            )
            .is_err()
        );
        assert!(
            write_report(
                &diff,
                "job-x",
                0,
                difflens_core::report::ReportLang::En,
                "1.0.0",
                "html",
                dir.path().join("nope/report.html").to_str().unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn validate_input_file_rejects_bad_paths() {
        assert!(validate_input_file("").is_err());
        assert!(validate_input_file("   ").is_err());
        assert!(validate_input_file("/nonexistent/nope.txt").is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(
            validate_input_file(dir.path().to_str().unwrap()).is_err(),
            "a directory is not a readable file"
        );
        let file = dir.path().join("doc.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(validate_input_file(file.to_str().unwrap()).is_ok());
    }

    #[test]
    fn validate_report_target_rejects_bad_paths() {
        let dir = tempfile::tempdir().unwrap();
        // Empty, relative, and wrong-extension targets are rejected.
        assert!(validate_report_target("", "html").is_err());
        assert!(validate_report_target("report.html", "html").is_err());
        let wrong_ext = dir.path().join("report.txt");
        assert!(validate_report_target(wrong_ext.to_str().unwrap(), "html").is_err());
        // Missing parent directory.
        let missing_parent = dir.path().join("nope").join("report.html");
        assert!(validate_report_target(missing_parent.to_str().unwrap(), "html").is_err());
        // A directory target (even with a matching extension).
        let weird_dir = dir.path().join("weird.html");
        std::fs::create_dir(&weird_dir).unwrap();
        assert!(validate_report_target(weird_dir.to_str().unwrap(), "html").is_err());
        // Valid targets for both formats; unknown format rejected.
        let ok_html = dir.path().join("out.html");
        assert!(validate_report_target(ok_html.to_str().unwrap(), "html").is_ok());
        let ok_md = dir.path().join("out.md");
        assert!(validate_report_target(ok_md.to_str().unwrap(), "markdown").is_ok());
        assert!(validate_report_target(ok_html.to_str().unwrap(), "pdf").is_err());
    }

    #[test]
    fn llm_download_cancel_flag_stops_the_progress_closure() {
        // Mirrors the wiring in `llm_install_server`/`llm_download_model`:
        // the job is registered with the core download manager and the
        // progress closure returns `!cancel`; flipping the flag via the
        // manager makes the closure abort (core then stops the transfer
        // with `CoreError::Cancelled`).
        let downloads = difflens_core::download::DownloadManager::new();
        let cancel = downloads.register("model-unit-test").unwrap();
        let mut calls = 0;
        let mut progress = |done: u64, total: Option<u64>| {
            let _ = (done, total);
            calls += 1;
            !cancel.load(Ordering::SeqCst)
        };
        assert!(progress(0, Some(10)), "runs while the flag is clear");
        assert!(downloads.cancel("model-unit-test").unwrap());
        assert!(!progress(1, Some(10)), "flagged closure must abort");
        downloads.finish(
            "model-unit-test",
            difflens_core::download::JobState::Cancelled,
            None,
        );
        assert!(!downloads.cancel("model-unit-test").unwrap());
        assert_eq!(calls, 2);
    }

    #[test]
    fn download_outcome_maps_results_to_terminal_states() {
        let ok: Result<PathBuf, difflens_core::CoreError> = Ok(PathBuf::from("m.bin"));
        let (state, job_state, error) = download_outcome(&ok);
        assert_eq!(state, "done");
        assert_eq!(job_state, difflens_core::download::JobState::Done);
        assert!(error.is_none());

        let cancelled: Result<PathBuf, difflens_core::CoreError> =
            Err(difflens_core::CoreError::Cancelled);
        let (state, job_state, error) = download_outcome(&cancelled);
        assert_eq!(state, "cancelled");
        assert_eq!(job_state, difflens_core::download::JobState::Cancelled);
        assert_eq!(error.as_deref(), Some("cancelled by user"));

        let failed: Result<PathBuf, difflens_core::CoreError> =
            Err(difflens_core::CoreError::Download("boom".into()));
        let (state, job_state, error) = download_outcome(&failed);
        assert_eq!(state, "failed");
        assert_eq!(job_state, difflens_core::download::JobState::Failed);
        assert!(error.is_some());
    }
}
