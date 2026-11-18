//! Shared application state: diff job registry with cancelation flags and
//! generation counter, plus the LLM sidecar/download managers.
//!
//! Thin bookkeeping only — all pipeline logic lives in `difflens-core`.
//! `run_diff` allocates a job id (`job-{generation}`), spawns the pipeline
//! on the async runtime and stores progress/results here; `get_diff` reads
//! snapshots; `cancel_diff` flips an atomic flag checked at stage
//! boundaries.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use difflens_core::DiffResult;
use difflens_core::download::DownloadManager;
use difflens_core::llm::sidecar::SidecarManager;
use difflens_core::store::Store;
use serde::Serialize;

/// Lifecycle of a diff job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Internal record for one diff job.
pub struct JobRecord {
    pub state: JobState,
    pub stage: String,
    pub percent: u8,
    /// Set by `cancel_diff`; polled at pipeline stage boundaries.
    pub cancel: Arc<AtomicBool>,
    pub result: Option<DiffResult>,
    pub error: Option<String>,
}

/// Serializable snapshot returned by `get_diff`.
#[derive(Debug, Clone, Serialize)]
pub struct JobSnapshot {
    pub job_id: String,
    pub state: JobState,
    pub stage: String,
    pub percent: u8,
    pub error: Option<String>,
    pub result: Option<DiffResult>,
}

impl JobSnapshot {
    fn from_record(job_id: &str, record: &JobRecord) -> JobSnapshot {
        JobSnapshot {
            job_id: job_id.to_string(),
            state: record.state,
            stage: record.stage.clone(),
            percent: record.percent,
            error: record.error.clone(),
            result: record.result.clone(),
        }
    }
}

/// Shared job map handed to the spawned pipeline task.
pub type JobMap = Arc<Mutex<HashMap<String, JobRecord>>>;

/// Lazily-opened session store handle (phase-08). `None` until
/// `setup` opens `app_data_dir/difflens.sqlite3`; every finished diff
/// job is persisted there (fire-and-forget).
pub type StoreHandle = Arc<Mutex<Option<Store>>>;

/// Root application state managed by Tauri. `diff` is Arc-shared so the
/// spawned job task can own a handle while commands keep theirs.
#[derive(Default)]
pub struct AppState {
    pub diff: Arc<DiffState>,
    /// LLM sidecar + download managers (phase-05). The sidecar manager
    /// kills its llama-server child on drop; the app also calls
    /// `shutdown()` on `RunEvent::Exit` for a deterministic teardown.
    pub llm: Arc<LlmState>,
    /// Session history store (phase-08), opened at app start.
    pub store: StoreHandle,
}

/// LLM feature state: download jobs, one active sidecar, streaming
/// explain jobs with cancel flags.
pub struct LlmState {
    /// Model/server download jobs (core `DownloadManager`).
    pub downloads: Arc<DownloadManager>,
    /// At most one llama-server child, with same-model reuse.
    pub sidecar: Arc<SidecarManager>,
    /// Explain (token-streaming) jobs with cancel flags.
    pub explain: Arc<ExplainJobs>,
}

impl Default for LlmState {
    fn default() -> Self {
        Self {
            downloads: Arc::new(DownloadManager::new()),
            sidecar: Arc::new(SidecarManager::new()),
            explain: Arc::new(ExplainJobs::default()),
        }
    }
}

/// Registry of running explain jobs (`explain-<n>`), mirroring DiffState's
/// job pattern.
#[derive(Default)]
pub struct ExplainJobs {
    generation: AtomicU64,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ExplainJobs {
    /// Allocate the next explain job id and its cancel flag.
    pub fn insert_running(&self) -> (String, Arc<AtomicBool>) {
        let id = format!(
            "explain-{}",
            self.generation.fetch_add(1, Ordering::SeqCst) + 1
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs
            .lock()
            .expect("explain job map poisoned")
            .insert(id.clone(), cancel.clone());
        (id, cancel)
    }

    /// Flag a running explain job cancelled; `true` when found.
    pub fn request_cancel(&self, job_id: &str) -> bool {
        let jobs = self.jobs.lock().expect("explain job map poisoned");
        if let Some(flag) = jobs.get(job_id) {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Drop the registry entry when the job finishes.
    pub fn remove(&self, job_id: &str) {
        self.jobs
            .lock()
            .expect("explain job map poisoned")
            .remove(job_id);
    }
}

pub struct DiffState {
    /// Monotonic generation counter; job ids derive from it.
    generation: AtomicU64,
    pub jobs: JobMap,
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl DiffState {
    /// Allocate the next job id (`job-1`, `job-2`, …).
    pub fn next_job_id(&self) -> String {
        let n = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        format!("job-{n}")
    }

    /// Insert a fresh running job; returns its cancel flag.
    pub fn insert_running(&self, job_id: &str) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        let record = JobRecord {
            state: JobState::Running,
            stage: "queued".to_string(),
            percent: 0,
            cancel: cancel.clone(),
            result: None,
            error: None,
        };
        self.jobs
            .lock()
            .expect("job map lock poisoned")
            .insert(job_id.to_string(), record);
        cancel
    }

    /// Snapshot a job (clones the result — DiffResult is moderate in size
    /// and returned once per poll).
    pub fn snapshot(&self, job_id: &str) -> Option<JobSnapshot> {
        let jobs = self.jobs.lock().expect("job map lock poisoned");
        jobs.get(job_id)
            .map(|r| JobSnapshot::from_record(job_id, r))
    }

    /// Update progress for a job (no-op for unknown ids).
    pub fn set_progress(&self, job_id: &str, stage: &str, percent: u8) {
        let mut jobs = self.jobs.lock().expect("job map lock poisoned");
        if let Some(record) = jobs.get_mut(job_id) {
            record.stage = stage.to_string();
            record.percent = percent;
        }
    }

    /// Finish a job with a result.
    pub fn finish(&self, job_id: &str, result: DiffResult) {
        let mut jobs = self.jobs.lock().expect("job map lock poisoned");
        if let Some(record) = jobs.get_mut(job_id) {
            record.state = JobState::Done;
            record.stage = "done".to_string();
            record.percent = 100;
            record.result = Some(result);
        }
    }

    /// Finish a job with an error (cancelation uses a dedicated state).
    pub fn fail(&self, job_id: &str, error: String) {
        let mut jobs = self.jobs.lock().expect("job map lock poisoned");
        if let Some(record) = jobs.get_mut(job_id) {
            record.state = JobState::Failed;
            record.stage = "failed".to_string();
            record.error = Some(error);
        }
    }

    /// Mark a job cancelled.
    pub fn mark_cancelled(&self, job_id: &str) {
        let mut jobs = self.jobs.lock().expect("job map lock poisoned");
        if let Some(record) = jobs.get_mut(job_id) {
            record.state = JobState::Cancelled;
            record.stage = "cancelled".to_string();
        }
    }

    /// Flip the cancel flag for a job; `true` when a running job was found.
    pub fn request_cancel(&self, job_id: &str) -> bool {
        let jobs = self.jobs.lock().expect("job map lock poisoned");
        match jobs.get(job_id) {
            Some(record) if record.state == JobState::Running => {
                record.cancel.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_lifecycle_transitions() {
        let state = DiffState::default();
        let id = state.next_job_id();
        assert_eq!(id, "job-0");
        let cancel = state.insert_running(&id);
        assert!(state.request_cancel(&id));
        assert!(cancel.load(Ordering::SeqCst));

        let id2 = state.next_job_id();
        assert_eq!(id2, "job-1");
        state.insert_running(&id2);
        let result = DiffResult {
            old_path: "a".into(),
            new_path: "b".into(),
            format: difflens_core::DocFormat::Txt,
            changes: vec![],
            stats: Default::default(),
            embeddings_used: false,
            ocr_used: false,
            audit: Vec::new(),
        };
        state.finish(&id2, result);
        let snap = state.snapshot(&id2).unwrap();
        assert_eq!(snap.state, JobState::Done);
        assert_eq!(snap.percent, 100);
        assert!(snap.result.is_some());

        state.fail(&id2, "boom".into());
        assert_eq!(state.snapshot(&id2).unwrap().state, JobState::Failed);

        state.mark_cancelled(&id2);
        assert_eq!(state.snapshot(&id2).unwrap().state, JobState::Cancelled);

        assert!(!state.request_cancel("missing"));
    }
}
