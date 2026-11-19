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
    /// Insertion-order stamp; pruning evicts the oldest finished jobs
    /// first (review P2-5).
    pub seq: u64,
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
    /// Allocate the next explain job id and its cancel flag. Poisoning
    /// surfaces as `Err` at the command boundary (P2-9).
    pub fn insert_running(&self) -> Result<(String, Arc<AtomicBool>), String> {
        let id = format!(
            "explain-{}",
            self.generation.fetch_add(1, Ordering::SeqCst) + 1
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs
            .lock()
            .map_err(|e| format!("explain job map poisoned: {e}"))?
            .insert(id.clone(), cancel.clone());
        Ok((id, cancel))
    }

    /// Flag a running explain job cancelled; `true` when found. Poisoning
    /// surfaces as `Err` (command boundary).
    pub fn request_cancel(&self, job_id: &str) -> Result<bool, String> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|e| format!("explain job map poisoned: {e}"))?;
        if let Some(flag) = jobs.get(job_id) {
            flag.store(true, Ordering::SeqCst);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Drop the registry entry when the job finishes. Fire-and-forget
    /// (called from the explain thread); poison is logged, never panicked
    /// on.
    pub fn remove(&self, job_id: &str) {
        let Ok(mut jobs) = self.jobs.lock() else {
            tracing::warn!("explain job map poisoned; cleanup of {job_id} skipped");
            return;
        };
        jobs.remove(job_id);
    }
}

pub struct DiffState {
    /// Monotonic generation counter; job ids derive from it.
    generation: AtomicU64,
    /// Monotonic insertion sequence for oldest-first pruning (P2-5).
    insertion: AtomicU64,
    pub jobs: JobMap,
}

/// Maximum number of diff jobs retained in memory (review P2-5). Every
/// finished job holds a full `DiffResult` blob; pruning on insert keeps a
/// long session bounded. Eviction prefers the oldest finished jobs
/// (Done/Failed/Cancelled); **Running jobs are never evicted**, so with
/// many concurrent jobs the map can transiently exceed the cap until the
/// next insert.
pub const MAX_JOBS: usize = 50;

impl Default for DiffState {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            insertion: AtomicU64::new(0),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Evict the oldest finished jobs until the map holds at most `MAX_JOBS`
/// records. Running jobs are never candidates; if every record is still
/// running, pruning stops (the cap is re-enforced on the next insert).
fn prune_locked(jobs: &mut HashMap<String, JobRecord>) {
    while jobs.len() > MAX_JOBS {
        let victim = jobs
            .iter()
            .filter(|(_, r)| r.state != JobState::Running)
            .min_by_key(|(_, r)| r.seq)
            .map(|(id, _)| id.clone());
        let Some(victim) = victim else { break };
        jobs.remove(&victim);
    }
}

impl DiffState {
    /// Allocate the next job id (`job-1`, `job-2`, …).
    pub fn next_job_id(&self) -> String {
        let n = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        format!("job-{n}")
    }

    /// Insert a fresh running job; returns its cancel flag. Prunes the
    /// oldest finished jobs when the map exceeds `MAX_JOBS` (P2-5).
    /// Poisoning the map lock surfaces as `Err` at the command boundary
    /// instead of panicking (P2-9).
    pub fn insert_running(&self, job_id: &str) -> Result<Arc<AtomicBool>, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let record = JobRecord {
            seq: self.insertion.fetch_add(1, Ordering::SeqCst),
            state: JobState::Running,
            stage: "queued".to_string(),
            percent: 0,
            cancel: cancel.clone(),
            result: None,
            error: None,
        };
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|e| format!("job map lock poisoned: {e}"))?;
        jobs.insert(job_id.to_string(), record);
        prune_locked(&mut jobs);
        Ok(cancel)
    }

    /// Snapshot a job (clones the result — DiffResult is moderate in size
    /// and returned once per poll). Poisoning surfaces as `Err`.
    pub fn snapshot(&self, job_id: &str) -> Result<Option<JobSnapshot>, String> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|e| format!("job map lock poisoned: {e}"))?;
        Ok(jobs
            .get(job_id)
            .map(|r| JobSnapshot::from_record(job_id, r)))
    }

    /// Update progress for a job (no-op for unknown ids). Fire-and-forget
    /// (called from the spawned job thread): lock poisoning is logged,
    /// never panicked on.
    pub fn set_progress(&self, job_id: &str, stage: &str, percent: u8) {
        let Ok(mut jobs) = self.jobs.lock() else {
            tracing::warn!("job map lock poisoned; progress update for {job_id} skipped");
            return;
        };
        if let Some(record) = jobs.get_mut(job_id) {
            record.stage = stage.to_string();
            record.percent = percent;
        }
    }

    /// Finish a job with a result. Fire-and-forget; poison is logged.
    pub fn finish(&self, job_id: &str, result: DiffResult) {
        let Ok(mut jobs) = self.jobs.lock() else {
            tracing::warn!("job map lock poisoned; finish for {job_id} skipped");
            return;
        };
        if let Some(record) = jobs.get_mut(job_id) {
            record.state = JobState::Done;
            record.stage = "done".to_string();
            record.percent = 100;
            record.result = Some(result);
        }
    }

    /// Finish a job with an error (cancelation uses a dedicated state).
    /// Fire-and-forget; poison is logged.
    pub fn fail(&self, job_id: &str, error: String) {
        let Ok(mut jobs) = self.jobs.lock() else {
            tracing::warn!("job map lock poisoned; failure for {job_id} skipped");
            return;
        };
        if let Some(record) = jobs.get_mut(job_id) {
            record.state = JobState::Failed;
            record.stage = "failed".to_string();
            record.error = Some(error);
        }
    }

    /// Mark a job cancelled. Fire-and-forget; poison is logged.
    pub fn mark_cancelled(&self, job_id: &str) {
        let Ok(mut jobs) = self.jobs.lock() else {
            tracing::warn!("job map lock poisoned; cancellation for {job_id} skipped");
            return;
        };
        if let Some(record) = jobs.get_mut(job_id) {
            record.state = JobState::Cancelled;
            record.stage = "cancelled".to_string();
        }
    }

    /// Flip the cancel flag for a job; `true` when a running job was
    /// found. Poisoning surfaces as `Err` (command boundary).
    pub fn request_cancel(&self, job_id: &str) -> Result<bool, String> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|e| format!("job map lock poisoned: {e}"))?;
        match jobs.get(job_id) {
            Some(record) if record.state == JobState::Running => {
                record.cancel.store(true, Ordering::SeqCst);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_result() -> DiffResult {
        DiffResult {
            old_path: "a".into(),
            new_path: "b".into(),
            format: difflens_core::DocFormat::Txt,
            changes: vec![],
            stats: Default::default(),
            embeddings_used: false,
            ocr_used: false,
            audit: Vec::new(),
        }
    }

    #[test]
    fn job_lifecycle_transitions() {
        let state = DiffState::default();
        let id = state.next_job_id();
        let cancel = state.insert_running(&id).unwrap();
        assert!(state.request_cancel(&id).unwrap());
        assert!(cancel.load(Ordering::SeqCst));

        let id2 = state.next_job_id();
        state.insert_running(&id2).unwrap();
        state.finish(&id2, tiny_result());
        let snap = state.snapshot(&id2).unwrap().unwrap();
        assert_eq!(snap.state, JobState::Done);
        assert_eq!(snap.percent, 100);
        assert!(snap.result.is_some());

        state.fail(&id2, "boom".into());
        assert_eq!(
            state.snapshot(&id2).unwrap().unwrap().state,
            JobState::Failed
        );

        state.mark_cancelled(&id2);
        assert_eq!(
            state.snapshot(&id2).unwrap().unwrap().state,
            JobState::Cancelled
        );

        assert!(!state.request_cancel("missing").unwrap());
    }

    #[test]
    fn job_map_prunes_oldest_finished_first_never_running() {
        // Review P2-5: inserting past MAX_JOBS evicts the oldest finished
        // jobs (Done/Failed/Cancelled); Running jobs survive.
        let state = DiffState::default();
        let keep_running = 5u64;
        for i in 0..60u64 {
            let id = format!("job-{i}");
            state.insert_running(&id).unwrap();
            if i != keep_running {
                state.finish(&id, tiny_result());
            }
        }
        let jobs = state.jobs.lock().unwrap();
        assert_eq!(jobs.len(), MAX_JOBS, "map must stay capped at MAX_JOBS");
        // The old running survivor is never evicted.
        assert!(jobs.contains_key("job-5"), "running survivor kept");
        assert_eq!(jobs["job-5"].state, JobState::Running);
        // The 10 oldest finished jobs went first (0-4, then 6-10 — the
        // running job-5 is skipped).
        assert!(!jobs.contains_key("job-0"));
        assert!(!jobs.contains_key("job-4"));
        assert!(!jobs.contains_key("job-6"));
        assert!(!jobs.contains_key("job-10"));
        assert!(jobs.contains_key("job-11"), "eviction stopped at the cap");
        assert!(jobs.contains_key("job-59"), "newest job kept");
    }

    #[test]
    fn job_map_stops_pruning_when_all_running() {
        // Running jobs are never evicted: with more running jobs than the
        // cap, pruning stops instead of dropping live jobs.
        let state = DiffState::default();
        for i in 0..(MAX_JOBS + 10) {
            state.insert_running(&format!("job-{i}")).unwrap();
        }
        let jobs = state.jobs.lock().unwrap();
        assert_eq!(jobs.len(), MAX_JOBS + 10, "no running job was evicted");
        assert!(jobs.contains_key("job-0"));
    }
}
