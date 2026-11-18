//! Tauri commands for the diff pipeline: `run_diff` / `get_diff` /
//! `cancel_diff` plus `diff://progress` event emission.
//!
//! Thin shell: the job body delegates to
//! [`difflens_core::pipeline::run_with_progress`]; this layer only tracks
//! jobs, emits progress and honors cancelation at pipeline stage
//! boundaries.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

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
    tauri::async_runtime::spawn(async move {
        execute_job(
            &job_id_task,
            &old_path,
            &new_path,
            &diff,
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
