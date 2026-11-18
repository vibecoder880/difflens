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

use crate::state::{AppState, DiffState, JobSnapshot, JobState};

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
#[tauri::command]
pub fn run_diff(
    old_path: String,
    new_path: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<JobSnapshot, String> {
    let diff = state.diff.clone();
    let job_id = diff.next_job_id();
    let cancel = diff.insert_running(&job_id);

    let job_id_task = job_id.clone();
    let app_task = app.clone();
    let diff_task = state.diff.clone();
    tauri::async_runtime::spawn(async move {
        execute_job(
            &job_id_task,
            &old_path,
            &new_path,
            &diff_task,
            cancel,
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

    diff.snapshot(&job_id)
        .ok_or_else(|| "job record vanished".to_string())
}

/// Fetch the current snapshot (status + result) of a diff job.
#[tauri::command]
pub fn get_diff(job_id: String, state: State<AppState>) -> Result<Option<JobSnapshot>, String> {
    Ok(state.diff.snapshot(&job_id))
}

/// Request cancelation. `true` when a running job was found and flagged;
/// the pipeline checks the flag at stage boundaries.
#[tauri::command]
pub fn cancel_diff(job_id: String, state: State<AppState>) -> Result<bool, String> {
    Ok(state.diff.request_cancel(&job_id))
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
        binary: avail
            .binary
            .map(|b| b.to_string_lossy().into_owned()),
        langs: avail.langs,
        has_vie: avail.has_vie,
    })
}

/// The synchronous job body — runs on the async runtime and is testable
/// headless (no `AppHandle` needed; events go through the `emit` closure).
fn execute_job(
    job_id: &str,
    old_path: &str,
    new_path: &str,
    diff: &DiffState,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    mut emit: impl FnMut(&'static str, u8),
) {
    let cancelled = || cancel.load(Ordering::SeqCst);
    let result = difflens_core::pipeline::run_with_progress(
        old_path,
        new_path,
        difflens_core::pipeline::default_embedder().as_ref(),
        // Phase-06: experimental OCR fallback, auto-enabled (runs only when
        // a scanned PDF is detected AND tesseract + vie are installed).
        &difflens_core::ocr::OcrOptions::default(),
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
        Ok(result) => diff.finish(job_id, result),
        Err(e) => {
            if cancelled() {
                diff.mark_cancelled(job_id);
            } else {
                diff.fail(job_id, e.to_string());
            }
        }
    }
    // Terminal progress event so listeners see the final state transition.
    let snapshot = diff.snapshot(job_id);
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

/// Model file name for a manifest entry (file component of its URL).
fn model_file(model: &difflens_core::download::manifest::ModelEntry) -> String {
    model.file.clone()
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
            let path = models_dir.join(model_file(m));
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

/// Download + install the manifest-pinned llama-server bundle under
/// `app_data/bin/llama-server-<tag>/`. Progress on `llm://download`.
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
    let job_id_thread = job_id.clone();
    std::thread::Builder::new()
        .name("difflens-install-server".into())
        .spawn(move || {
            let emit = |p: LlmDownloadPayload| {
                let _ = app_task.emit(LLM_DOWNLOAD_EVENT, &p);
            };
            let result = difflens_core::download::download_file(&spec, &mut |done, total| {
                let percent = total
                    .map(|t| ((done as f64 / t.max(1) as f64) * 100.0).min(100.0) as u8)
                    .unwrap_or(0);
                emit(LlmDownloadPayload {
                    job_id: job_id_thread.clone(),
                    kind: "server".into(),
                    model_id: String::new(),
                    state: "running".into(),
                    bytes_done: done,
                    bytes_total: total,
                    percent,
                    error: None,
                });
                true
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

            let error = match &result {
                Ok(_) => {
                    // Reclaim disk: the verified archive is redundant.
                    let _ = std::fs::remove_file(&archive_path);
                    None
                }
                Err(difflens_core::CoreError::Cancelled) => Some("cancelled by user".into()),
                Err(e) => Some(e.to_string()),
            };
            let state = if error.is_some() {
                if error.as_deref() == Some("cancelled by user") {
                    "cancelled"
                } else {
                    "failed"
                }
            } else {
                "done"
            };
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
/// `llm://download`; cancelable via `llm_cancel`.
#[tauri::command]
pub fn llm_download_model(model_id: String, app: AppHandle) -> Result<String, String> {
    let manifest = difflens_core::download::manifest::LlmManifest::embedded();
    let model = manifest
        .model(&model_id)
        .cloned()
        .ok_or_else(|| format!("unknown model id {model_id:?}"))?;
    let (_, models_dir) = app_paths(&app)?;
    let dest = models_dir.join(model_file(&model));

    let mut spec =
        difflens_core::download::DownloadSpec::new(model.url.clone(), dest, model.sha256.clone());
    spec.expected_bytes = Some(model.bytes);

    let app_task = app.clone();
    let job_id = format!("model-{}", model.id);
    let job_id_thread = job_id.clone();
    let model_id = model.id.clone();
    std::thread::Builder::new()
        .name(format!("difflens-model-{model_id}"))
        .spawn(move || {
            let emit = |p: LlmDownloadPayload| {
                let _ = app_task.emit(LLM_DOWNLOAD_EVENT, &p);
            };
            let result = difflens_core::download::download_file(&spec, &mut |done, total| {
                let percent = total
                    .map(|t| ((done as f64 / t.max(1) as f64) * 100.0).min(100.0) as u8)
                    .unwrap_or(0);
                emit(LlmDownloadPayload {
                    job_id: job_id_thread.clone(),
                    kind: "model".into(),
                    model_id: model_id.clone(),
                    state: "running".into(),
                    bytes_done: done,
                    bytes_total: total,
                    percent,
                    error: None,
                });
                true
            });
            let (state, error) = match result {
                Ok(_) => ("done", None),
                Err(difflens_core::CoreError::Cancelled) => {
                    ("cancelled", Some("cancelled by user".into()))
                }
                Err(e) => ("failed", Some(e.to_string())),
            };
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

    // Resolve the change from the stored diff result.
    let diff = state.diff.clone();
    let snapshot = diff
        .snapshot(&diff_job_id)
        .ok_or_else(|| format!("unknown diff job {diff_job_id:?}"))?;
    let result = snapshot.result.ok_or_else(|| {
        format!(
            "diff job {diff_job_id:?} has no result yet (state {:?})",
            snapshot.state
        )
    })?;
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
        .map(|m| (m.clone(), models_dir.join(model_file(m))))
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

    let (job_id, cancel) = state.llm.explain.insert_running();
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

/// Cancel an LLM job: model/server download (`dl-*`/`model-*`/`server-*`
/// ids) or a running explain job (`explain-*` ids). `true` when found.
#[tauri::command]
pub fn llm_cancel(job_id: String, state: State<AppState>) -> Result<bool, String> {
    if job_id.starts_with("explain-") {
        return Ok(state.llm.explain.request_cancel(&job_id));
    }
    if job_id.starts_with("dl-") {
        return Ok(state.llm.downloads.cancel(&job_id));
    }
    // Downloads started through the LLM commands use their own id scheme
    // but flow through the same core manager; also try it directly.
    if state.llm.downloads.cancel(&job_id) {
        return Ok(true);
    }
    Ok(false)
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
        let cancel = diff.insert_running(&job_id);
        let a = write_temp("a.txt", "# Contract\n\nPayment within 30 days.\n");
        let b = write_temp("b.txt", "# Contract\n\nPayment within 45 days.\n");

        let mut events: Vec<(String, u8)> = Vec::new();
        execute_job(&job_id, &a, &b, &diff, cancel, |stage, percent| {
            events.push((stage.to_string(), percent));
        });

        let snap = diff.snapshot(&job_id).expect("job present");
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
        let cancel = diff.insert_running(&job_id);
        cancel.store(true, Ordering::SeqCst);
        let a = write_temp("c.txt", "one\n");
        let b = write_temp("d.txt", "two\n");

        execute_job(&job_id, &a, &b, &diff, cancel, |_, _| {});
        let snap = diff.snapshot(&job_id).unwrap();
        assert_eq!(snap.state, JobState::Cancelled);
    }

    #[test]
    fn failing_input_marks_job_failed() {
        let diff = DiffState::default();
        let job_id = diff.next_job_id();
        let cancel = diff.insert_running(&job_id);
        execute_job(
            &job_id,
            "/nonexistent/a.txt",
            "/nonexistent/b.txt",
            &diff,
            cancel,
            |_, _| {},
        );
        let snap = diff.snapshot(&job_id).unwrap();
        assert_eq!(snap.state, JobState::Failed);
        assert!(
            snap.error
                .as_deref()
                .unwrap_or_default()
                .contains("cannot read")
        );
    }
}
