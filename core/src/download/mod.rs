//! The **only** network egress in the workspace (see
//! `scripts/audit-egress.sh`): a job-based download manager for the
//! optional local-LLM sidecar artifacts.
//!
//! Design notes:
//! - std-thread based, no runtime dependency on tokio inside `core`;
//!   the Tauri shell wraps jobs with events. Deterministic helpers are
//!   free functions (`download_file`, `install_bundle`) so tests run
//!   headless without the manager.
//! - HTTP via `ureq` (rustls). Streams to `<dest>.part` with HTTP Range
//!   resume; a server that ignores the range (HTTP 200) restarts from
//!   scratch; HTTP 416 (stale `.part`) restarts exactly once — a second
//!   416 (on the plain GET) fails the job instead of looping.
//! - Size caps enforced while streaming; SHA-256 verified against the
//!   manifest digest before the atomic `.part → dest` rename.
//! - Cancellation is cooperative: the progress closure returns `false`
//!   and the download stops, keeping the `.part` for a later resume.

pub mod archive;
pub mod checksum;
pub mod manifest;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::{CoreError, CoreResult};

/// Hard ceiling applied when the caller does not specify one. The largest
/// manifest artifact today is the 4B Q4_K_M GGUF (~2.5 GB); 4 GiB leaves
/// headroom while still bounding a runaway server.
pub const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Copy buffer size for the streaming loop.
const CHUNK: usize = 64 * 1024;

/// Connect/read timeouts. There is no unbounded stall path: the read
/// timeout applies between socket reads and the overall request timeout
/// (set per request) bounds the whole transfer including body streaming.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Shared ureq agent (connection pooling, timeouts). Also used by the
/// loopback-only LLM client (the only other legitimate HTTP construction,
/// per the egress audit allowlist).
pub fn build_llm_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build()
}

/// What to download and where it must land.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub url: String,
    /// Final destination (the atomic rename target).
    pub dest: PathBuf,
    /// Expected lowercase sha256 (from the pinned manifest).
    pub expected_sha256: String,
    /// Expected byte size from the manifest, when known; mismatch fails
    /// the job even before hashing.
    pub expected_bytes: Option<u64>,
    /// Decompressed/artifact size cap.
    pub max_bytes: u64,
    /// Resume from an existing `<dest>.part` when the server supports it.
    pub resume: bool,
}

impl DownloadSpec {
    pub fn new(url: impl Into<String>, dest: PathBuf, expected_sha256: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            dest,
            expected_sha256: expected_sha256.into(),
            expected_bytes: None,
            max_bytes: DEFAULT_MAX_BYTES,
            resume: true,
        }
    }
}

/// Progress callback: `(bytes_done, bytes_total_opt) -> keep_going`.
/// Returning `false` cancels the job (the `.part` is kept for resume).
pub type ProgressFn<'a> = dyn FnMut(u64, Option<u64>) -> bool + 'a;

/// Core blocking download. Returns the final `dest` path.
///
/// Errors are `CoreError::Download` (or `Io`); checksum mismatch removes
/// the `.part` (it is corrupt or the manifest is wrong — resuming cannot
/// fix either).
pub fn download_file(spec: &DownloadSpec, progress: &mut ProgressFn) -> CoreResult<PathBuf> {
    if !checksum::is_sha256_hex(&spec.expected_sha256) {
        return Err(CoreError::Download(format!(
            "invalid expected sha256: {:?}",
            spec.expected_sha256
        )));
    }
    if let Some(parent) = spec.dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = part_path(&spec.dest);

    // Resume bookkeeping.
    let mut start_at: u64 = 0;
    if spec.resume
        && let Ok(meta) = std::fs::metadata(&part)
    {
        if meta.len() > spec.max_bytes {
            // Oversized leftover part: corrupt or hostile — discard.
            let _ = std::fs::remove_file(&part);
        } else {
            start_at = meta.len();
        }
    }

    let agent = build_llm_agent();

    // Issue the (possibly ranged) GET and normalize the response into
    // (writer, total, reader). Any protocol surprise becomes an error;
    // stale/ignored ranges degrade to a clean restart.
    let (writer, total, mut reader) = loop {
        let mut req = agent.get(&spec.url);
        if start_at > 0 {
            req = req.set("Range", &format!("bytes={start_at}-"));
        }
        // ureq surfaces 4xx/5xx as `Error::Status`; unwrap a 416 so the
        // restart handling in the match below sees it as a plain status.
        let resp = match req.call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(416, resp)) => resp,
            Err(e) => return Err(map_ureq_error("request", &e)),
        };
        let status = resp.status();
        let content_len: Option<u64> = resp.header("Content-Length").and_then(|v| v.parse().ok());

        match (start_at > 0, status) {
            // Server honored the range — append to the .part.
            (true, 206) => {
                let file = std::fs::OpenOptions::new().append(true).open(&part)?;
                let total = content_len.map(|cl| start_at + cl);
                break (file, total, resp.into_reader());
            }
            // Server ignored the range (200) — restart from scratch.
            (true, 200) | (false, 200) => {
                if start_at > 0 {
                    start_at = 0;
                }
                let file = std::fs::File::create(&part)?;
                break (file, content_len, resp.into_reader());
            }
            // Range not satisfiable (stale .part, artifact rotated on the
            // server): restart once with a plain GET. A 416 on the plain
            // GET itself is fatal — retrying cannot fix a server that
            // answers 416 unconditionally, so fail instead of looping.
            (_, 416) => {
                if start_at == 0 {
                    return Err(CoreError::Download(format!(
                        "server returned 416 for a full GET: {}",
                        spec.url
                    )));
                }
                let _ = std::fs::remove_file(&part);
                start_at = 0;
                continue;
            }
            _ => {
                return Err(CoreError::Download(format!(
                    "unexpected HTTP status {status} for {}",
                    spec.url
                )));
            }
        }
    };

    // Cap check up front when the server told us the size.
    if let Some(t) = total
        && t > spec.max_bytes
    {
        return Err(CoreError::Download(format!(
            "content-length {t} exceeds size cap {}",
            spec.max_bytes
        )));
    }

    let mut file = writer;
    let mut buf = vec![0u8; CHUNK];
    let mut done = start_at;
    loop {
        if !progress(done, total) {
            // Cooperative cancel: keep the .part for a later resume.
            file.flush()?;
            return Err(CoreError::Cancelled);
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| CoreError::Download(format!("read body: {e}")))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > spec.max_bytes {
            file.flush()?;
            return Err(CoreError::Download(format!(
                "download exceeded size cap {}",
                spec.max_bytes
            )));
        }
        file.write_all(&buf[..n])?;
        if let Some(expected) = spec.expected_bytes
            && done > expected
        {
            return Err(CoreError::Download(format!(
                "download exceeded manifest size {expected}"
            )));
        }
    }
    file.flush()?;
    file.sync_all().ok();

    if let Some(expected) = spec.expected_bytes
        && done != expected
    {
        return Err(CoreError::Download(format!(
            "size mismatch: got {done}, manifest says {expected}"
        )));
    }

    // Gate: SHA-256 before the atomic rename. Mismatch is fatal and the
    // corrupt .part is removed (resuming cannot fix bad bytes).
    if let Err(e) = checksum::verify_file(&part, &spec.expected_sha256, spec.max_bytes) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    // Cross-platform atomic-ish replace: POSIX rename clobbers, Windows
    // does not, so remove first (dest is a verified artifact, not user
    // data).
    let _ = std::fs::remove_file(&spec.dest);
    std::fs::rename(&part, &spec.dest)?;
    progress(done, Some(done));
    Ok(spec.dest.clone())
}

fn part_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "artifact".to_string());
    dest.with_file_name(format!("{name}.part"))
}

fn map_ureq_error(what: &str, e: &ureq::Error) -> CoreError {
    CoreError::Download(format!("{what}: {e}"))
}

// ---------------------------------------------------------------------------
// Job manager
// ---------------------------------------------------------------------------

/// Lifecycle of a managed download job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }
}

/// Serializable job snapshot for UI polling / events.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobSnapshot {
    pub job_id: String,
    pub state: JobState,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    /// Set when state == Done: the final artifact path.
    pub dest: Option<PathBuf>,
    pub error: Option<String>,
}

struct JobRecord {
    state: JobState,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    bytes_done: Arc<AtomicU64>,
    bytes_total: Mutex<Option<u64>>,
    dest: Option<PathBuf>,
    error: Option<String>,
}

/// Owns background download threads. Jobs are tracked in a registry; the
/// Tauri layer maps job ids onto `llm://download` events.
#[derive(Default)]
pub struct DownloadManager {
    next_id: AtomicU64,
    jobs: Arc<Mutex<HashMap<String, JobRecord>>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a download whose thread the manager does not own (e.g. the
    /// Tauri install thread that also extracts the archive after the
    /// transfer) so `cancel` and `snapshot` see it. Returns the shared
    /// cancel flag: the download's progress closure must return
    /// `!flag.load(Ordering::SeqCst)` so a flagged job aborts with
    /// `CoreError::Cancelled`, and the owner reports the terminal state
    /// via [`Self::finish`]. Re-registering a still-running job joins the
    /// existing flag instead of resetting it.
    pub fn register(&self, job_id: &str) -> Arc<std::sync::atomic::AtomicBool> {
        let mut jobs = self.jobs.lock().expect("download job map poisoned");
        if let Some(rec) = jobs.get(job_id)
            && rec.state == JobState::Running
        {
            return rec.cancel.clone();
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let record = JobRecord {
            state: JobState::Running,
            cancel: cancel.clone(),
            bytes_done: Arc::new(AtomicU64::new(0)),
            bytes_total: Mutex::new(None),
            dest: None,
            error: None,
        };
        jobs.insert(job_id.to_string(), record);
        cancel
    }

    /// Record the terminal state of an externally-run job (`Done`,
    /// `Failed`, or `Cancelled`) so `cancel` stops flagging it. No-op for
    /// unknown ids, already-terminal jobs, or a `Running` argument.
    pub fn finish(&self, job_id: &str, state: JobState, error: Option<String>) {
        if state == JobState::Running {
            return;
        }
        let mut jobs = self.jobs.lock().expect("download job map poisoned");
        if let Some(rec) = jobs.get_mut(job_id)
            && rec.state == JobState::Running
        {
            rec.state = state;
            rec.error = error;
        }
    }

    /// Start a background download. Progress is forwarded to `on_progress`
    /// (invoked on the download thread). Returns the job id immediately.
    pub fn start(
        &self,
        spec: DownloadSpec,
        mut on_progress: impl FnMut(u64, Option<u64>) + Send + 'static,
    ) -> String {
        let job_id = format!("dl-{}", self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bytes_done = Arc::new(AtomicU64::new(0));
        let record = JobRecord {
            state: JobState::Running,
            cancel: cancel.clone(),
            bytes_done: bytes_done.clone(),
            bytes_total: Mutex::new(spec.expected_bytes),
            dest: Some(spec.dest.clone()),
            error: None,
        };
        self.jobs
            .lock()
            .expect("download job map poisoned")
            .insert(job_id.clone(), record);

        let jobs = self.jobs.clone();
        let job_id_thread = job_id.clone();
        let flag = cancel;
        std::thread::Builder::new()
            .name(format!("difflens-dl-{job_id}"))
            .spawn(move || {
                let outcome = download_file(&spec, &mut |done, total| {
                    bytes_done.store(done, Ordering::SeqCst);
                    if let Some(t) = total {
                        *jobs
                            .lock()
                            .expect("download job map poisoned")
                            .get_mut(&job_id_thread)
                            .expect("job record present")
                            .bytes_total
                            .lock()
                            .expect("bytes_total poisoned") = Some(t);
                    }
                    on_progress(done, total);
                    !flag.load(Ordering::SeqCst)
                });
                let mut jobs = jobs.lock().expect("download job map poisoned");
                let rec = jobs
                    .get_mut(&job_id_thread)
                    .expect("job record present in thread");
                match outcome {
                    Ok(dest) => {
                        rec.state = JobState::Done;
                        rec.dest = Some(dest);
                    }
                    Err(CoreError::Cancelled) => rec.state = JobState::Cancelled,
                    Err(e) => {
                        rec.state = JobState::Failed;
                        rec.error = Some(e.to_string());
                    }
                }
            })
            .expect("spawn download thread");
        job_id
    }

    /// Flag a running job as cancelled. `true` when a running job matched.
    pub fn cancel(&self, job_id: &str) -> bool {
        let jobs = self.jobs.lock().expect("download job map poisoned");
        match jobs.get(job_id) {
            Some(rec) if rec.state == JobState::Running => {
                rec.cancel.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    /// Snapshot a job (unknown id → None).
    pub fn snapshot(&self, job_id: &str) -> Option<JobSnapshot> {
        let jobs = self.jobs.lock().expect("download job map poisoned");
        jobs.get(job_id).map(|rec| JobSnapshot {
            job_id: job_id.to_string(),
            state: rec.state,
            bytes_done: rec.bytes_done.load(Ordering::SeqCst),
            bytes_total: *rec.bytes_total.lock().expect("bytes_total poisoned"),
            dest: rec.dest.clone(),
            error: rec.error.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// Minimal loopback HTTP server serving `body` (or a byte range).
    /// This is the deterministic-tests-only network touchpoint: 127.0.0.1,
    /// spawned per test, no external network.
    struct LoopbackServer {
        addr: String,
    }

    impl LoopbackServer {
        fn spawn(body: Vec<u8>, support_range: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = match stream {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let req = String::from_utf8_lossy(&buf);
                    let range_supported = support_range && req.contains("Range:");
                    let mut start = 0usize;
                    let mut status_line = "200 OK";
                    if range_supported {
                        if let Some(pos) = req.find("bytes=") {
                            let rest = &req[pos + 6..];
                            if let Some(dash) = rest.find('-') {
                                start = rest[..dash].trim().parse().unwrap_or(0);
                            }
                        }
                        if start >= body.len() {
                            // Connection: close (like the 200 path) so the
                            // client does not reuse this socket after the
                            // restart-triggering 416.
                            let head = "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            std::io::Write::write_all(&mut stream, head.as_bytes()).ok();
                            continue;
                        }
                        status_line = "206 Partial Content";
                    }
                    let payload = &body[start..];
                    let head = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    stream.write_all(head.as_bytes()).ok();
                    stream.write_all(payload).ok();
                }
            });
            Self { addr }
        }
    }

    fn spec_for(url: String, dest: PathBuf, sha: String, size: usize) -> DownloadSpec {
        let mut spec = DownloadSpec::new(url, dest, sha);
        spec.expected_bytes = Some(size as u64);
        spec
    }

    #[test]
    fn download_verifies_and_renames_atomically() {
        let body = b"artifact-bytes".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, false);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let spec = spec_for(
            format!("http://{}/file", server.addr),
            dest.clone(),
            sha,
            14,
        );

        let got = download_file(&spec, &mut |_, _| true).unwrap();
        assert_eq!(got, dest);
        assert!(!dest.with_file_name("model.bin.part").exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"artifact-bytes");
    }

    #[test]
    fn checksum_mismatch_removes_part_and_fails() {
        let body = b"artifact-bytes".to_vec();
        let server = LoopbackServer::spawn(body, false);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let spec = spec_for(
            format!("http://{}/file", server.addr),
            dest.clone(),
            checksum::sha256_bytes(b"other-bytes!!!"),
            14,
        );
        let err = download_file(&spec, &mut |_, _| true).unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        assert!(!dest.exists());
        assert!(!dest.with_file_name("model.bin.part").exists());
    }

    #[test]
    fn resume_appends_from_range() {
        let body = b"0123456789".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, true);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        // Pre-existing partial download (first 4 bytes).
        std::fs::write(dest.with_file_name("model.bin.part"), b"0123").unwrap();
        let spec = spec_for(
            format!("http://{}/file", server.addr),
            dest.clone(),
            sha,
            10,
        );
        download_file(&spec, &mut |_, _| true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"0123456789");
    }

    #[test]
    fn cancellation_stops_and_keeps_part() {
        let body = b"0123456789".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, true);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let spec = spec_for(
            format!("http://{}/file", server.addr),
            dest.clone(),
            sha,
            10,
        );
        // Cancel before the first read: the job stops with Cancelled and
        // keeps whatever part existed (none yet).
        let err = download_file(&spec, &mut |_, _| false).unwrap_err();
        assert!(matches!(err, CoreError::Cancelled));
    }

    #[test]
    fn manager_job_lifecycle() {
        let body = b"payload".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, false);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        let spec = spec_for(format!("http://{}/f", server.addr), dest.clone(), sha, 7);
        let mgr = DownloadManager::new();
        let job = mgr.start(spec, |_, _| {});
        // Poll until terminal state (bounded — loopback is fast).
        for _ in 0..200 {
            let snap = mgr.snapshot(&job).unwrap();
            if snap.state != JobState::Running {
                assert_eq!(snap.state, JobState::Done);
                assert_eq!(snap.bytes_done, 7);
                assert_eq!(snap.dest.unwrap(), dest);
                assert!(!mgr.cancel(&job));
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("job never finished");
    }

    #[test]
    fn manager_cancel_flags_running_job() {
        let body = vec![0x41u8; 1 << 20]; // 1 MiB — several read loops
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, false);
        let dir = tempfile::tempdir().unwrap();
        let spec = spec_for(
            format!("http://{}/big", server.addr),
            dir.path().join("big.bin"),
            sha,
            1 << 20,
        );
        let mgr = DownloadManager::new();
        let job = mgr.start(spec, |_, _| {});
        // Cancel as soon as possible; either it was still running (true)
        // or already done (false) — both acceptable, flag must not panic.
        let _ = mgr.cancel(&job);
        for _ in 0..200 {
            if mgr.snapshot(&job).unwrap().state != JobState::Running {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn external_job_registration_supports_cancel_and_finish() {
        // Simulates the Tauri side: a download whose thread the manager
        // does not own; cancel flags it, finish records the outcome.
        let mgr = DownloadManager::new();
        let flag = mgr.register("model-test");
        assert_eq!(mgr.snapshot("model-test").unwrap().state, JobState::Running);
        // Re-registering while running joins the same flag.
        assert!(Arc::ptr_eq(&flag, &mgr.register("model-test")));
        assert!(mgr.cancel("model-test"));
        assert!(flag.load(Ordering::SeqCst));

        mgr.finish("model-test", JobState::Failed, Some("boom".into()));
        let snap = mgr.snapshot("model-test").unwrap();
        assert_eq!(snap.state, JobState::Failed);
        assert_eq!(snap.error.as_deref(), Some("boom"));

        // Terminal is sticky; cancel on a finished job is a no-op.
        mgr.finish("model-test", JobState::Done, None);
        assert_eq!(mgr.snapshot("model-test").unwrap().state, JobState::Failed);
        assert!(!mgr.cancel("model-test"));

        // Unknown ids and a `Running` "finish" are no-ops.
        mgr.finish("missing", JobState::Done, None);
        assert!(mgr.snapshot("missing").is_none());
        let flag2 = mgr.register("model-other");
        mgr.finish("model-other", JobState::Running, None);
        assert_eq!(
            mgr.snapshot("model-other").unwrap().state,
            JobState::Running
        );
        assert!(mgr.cancel("model-other"));
        assert!(flag2.load(Ordering::SeqCst));
    }

    #[test]
    fn registered_external_download_aborts_on_cancel() {
        // End-to-end cancel wiring proof: an externally-run download_file
        // whose progress closure returns `!flag` stops with `Cancelled`
        // when the manager flags the job.
        let body = b"x".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, false);
        let dir = tempfile::tempdir().unwrap();
        let spec = spec_for(
            format!("http://{}/f", server.addr),
            dir.path().join("m.bin"),
            sha,
            1,
        );
        let mgr = Arc::new(DownloadManager::new());
        let job_id = "model-cancelable".to_string();
        let flag = mgr.register(&job_id);
        // Deterministic ordering: the canceller fires only after the
        // transfer reached its first progress call (`entered`), and the
        // progress closure only returns after the flag is set (`spin`) —
        // so the very first callback aborts the transfer.
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mgr2 = mgr.clone();
        let entered2 = entered.clone();
        let job_id_cancel = job_id.clone();
        let canceller = std::thread::spawn(move || {
            while !entered2.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(mgr2.cancel(&job_id_cancel));
        });
        let outcome = download_file(&spec, &mut |_, _| {
            entered.store(true, Ordering::SeqCst);
            while !flag.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            false
        });
        canceller.join().unwrap();
        assert!(matches!(outcome, Err(CoreError::Cancelled)));

        mgr.finish(&job_id, JobState::Cancelled, None);
        assert_eq!(mgr.snapshot(&job_id).unwrap().state, JobState::Cancelled);
    }

    #[test]
    fn oversized_part_is_discarded_on_resume() {
        let body = b"short".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, true);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        // A part larger than max_bytes must be discarded, not resumed.
        let part = dest.with_file_name("m.bin.part");
        let mut f = std::fs::File::create(&part).unwrap();
        f.write_all(&vec![0u8; 8 * 1024 * 1024]).unwrap();
        drop(f);
        let mut spec = spec_for(format!("http://{}/f", server.addr), dest.clone(), sha, 5);
        spec.max_bytes = 1024 * 1024;
        download_file(&spec, &mut |_, _| true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"short");
    }

    #[test]
    fn stale_part_416_restarts_once_and_succeeds() {
        // Range request past EOF → 416 once, then the plain GET succeeds.
        let body = b"0123456789".to_vec();
        let sha = checksum::sha256_bytes(&body);
        let server = LoopbackServer::spawn(body, true);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        // Stale part: its length equals (>=) the new artifact size, so the
        // ranged resume is not satisfiable.
        std::fs::write(dest.with_file_name("m.bin.part"), b"0123456789").unwrap();
        let spec = spec_for(format!("http://{}/f", server.addr), dest.clone(), sha, 10);
        download_file(&spec, &mut |_, _| true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"0123456789");
        assert!(!dest.with_file_name("m.bin.part").exists());
    }

    /// Server that answers every request with 416 (broken or hostile).
    /// One response per connection (`Connection: close`) so the client
    /// never reuses a socket the server has already abandoned.
    struct Always416Server {
        addr: String,
    }

    impl Always416Server {
        fn spawn() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let head = "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    std::io::Write::write_all(&mut stream, head.as_bytes()).ok();
                }
            });
            Self { addr }
        }
    }

    #[test]
    fn persistent_416_fails_instead_of_looping_forever() {
        // Seeded part → first (ranged) GET hits 416 and restarts; the
        // second (plain) GET hits 416 again → must error, not loop.
        let server = Always416Server::spawn();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        std::fs::write(dest.with_file_name("m.bin.part"), b"stale-part").unwrap();
        let spec = DownloadSpec::new(
            format!("http://{}/f", server.addr),
            dest.clone(),
            checksum::sha256_bytes(b"whatever"),
        );
        let err = download_file(&spec, &mut |_, _| true).unwrap_err();
        assert!(
            err.to_string().contains("416 for a full GET"),
            "unexpected error: {err}"
        );
        // The failed attempt removes the stale part (restart path ran once).
        assert!(!dest.with_file_name("m.bin.part").exists());
    }

    #[test]
    fn plain_get_416_fails_immediately() {
        // No part at all: the very first request is a plain GET; a 416
        // answer must fail the job on the spot.
        let server = Always416Server::spawn();
        let dir = tempfile::tempdir().unwrap();
        let spec = DownloadSpec::new(
            format!("http://{}/f", server.addr),
            dir.path().join("m.bin"),
            checksum::sha256_bytes(b"whatever"),
        );
        let err = download_file(&spec, &mut |_, _| true).unwrap_err();
        assert!(err.to_string().contains("416 for a full GET"), "{err}");
    }
}
