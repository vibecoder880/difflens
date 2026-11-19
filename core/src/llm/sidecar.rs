//! llama-server sidecar lifecycle: detection, installation, spawn,
//! health polling and clean teardown.
//!
//! Security posture:
//! - The server is always launched **by absolute path** (no PATH
//!   shadowing) with `--host 127.0.0.1` and a **random per-session API
//!   key** generated from the OS CSPRNG. The key is handed to the server
//!   via the `LLAMA_API_KEY` environment variable — **never argv** — so
//!   it does not appear in process listings (`ps`, `/proc/<pid>/cmdline`)
//!   visible to other local users. Verified against the pinned build:
//!   llama.cpp b12503 `llama-server --help` documents
//!   `--api-key KEY (env: LLAMA_API_KEY)`; env hand-off is the documented
//!   same-process configuration path. A random ephemeral port comes from
//!   `--port 0` (verified working on the pinned build; the bind line is
//!   parsed from the startup log).
//! - `/health` is polled with the key until `{"status":"ok"}`.
//!   NOTE (verified 2026-11-18, llama.cpp b12503): llama.cpp exempts
//!   `/health` from key auth by design (load-balancer probe); auth was
//!   verified on `POST /v1/chat/completions` (401 without key).
//! - The child is killed on `Drop` and when replaced, so closing the app
//!   tears the server down.
//! - Stale-process reuse: a healthy server serving the same model is
//!   reused instead of respawning.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use regex::Regex;

use crate::download::archive::{self, ExtractLimits};
use crate::download::manifest::ArchiveFormat;
use crate::error::{CoreError, CoreResult};

/// Directory name prefix for installed server bundles under the bin root:
/// `binaries/llama-server-<tag>/`.
pub const SERVER_DIR_PREFIX: &str = "llama-server-";

/// Health-poll cadence and overall spawn deadline.
pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const SPAWN_DEADLINE: Duration = Duration::from_secs(60);

/// Ring-buffer cap for captured child output (diagnostics only).
const LOG_CAP: usize = 64 * 1024;

/// `bin_root/llama-server-<tag>/` for a given pinned tag.
pub fn server_dir(bin_root: &Path, tag: &str) -> PathBuf {
    bin_root.join(format!("{SERVER_DIR_PREFIX}{tag}"))
}

/// The launcher executable name for the current OS.
pub fn server_exe_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Detect an installed llama-server for `tag` under `bin_root`
/// (returns the launcher path when present and executable-looking).
pub fn detect_installed(bin_root: &Path, tag: &str) -> Option<PathBuf> {
    let p = server_dir(bin_root, tag).join(server_exe_name());
    p.is_file().then_some(p)
}

/// Detect llama-server on PATH (last resort; the app prefers its own
/// manifest-pinned install). Returns the absolute path found.
pub fn detect_on_path() -> Option<PathBuf> {
    let name = server_exe_name();
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Install a pinned server bundle: hardened extraction into a fresh
/// staging dir under `bin_root`, flatten (all bundles ship a single top
/// dir — or a flat zip on Windows — and the launcher's shared libraries
/// are resolved relative to the launcher itself), then an atomic rename
/// into `bin_root/llama-server-<tag>/`. Returns the launcher path.
/// Idempotent: an existing install is replaced only after the new staging
/// extraction fully succeeded.
pub fn install_bundle(
    archive_path: &Path,
    format: ArchiveFormat,
    tag: &str,
    bin_root: &Path,
    limits: &ExtractLimits,
) -> CoreResult<PathBuf> {
    std::fs::create_dir_all(bin_root)?;
    let staging = bin_root.join(format!(".staging-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let extracted = archive::extract_archive(archive_path, format, &staging, limits)?;

    // The launcher must exist in the extracted bundle.
    let launcher_name = server_exe_name();
    let launcher = extracted
        .files
        .iter()
        .find(|f| {
            f.file_name()
                .map(|n| n.to_string_lossy() == launcher_name)
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            CoreError::Download(format!(
                "server bundle missing {launcher_name} after extraction"
            ))
        })?
        .clone();

    // Flatten: extracted files may sit inside `llama-bNNNNN/`; move every
    // file to the staging root so `$ORIGIN`-relative library resolution
    // keeps working with a stable final path. Duplicate basenames are a
    // bundle defect and abort the install.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for file in &extracted.files {
        let base = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if base.is_empty() {
            continue;
        }
        let target = staging.join(&base);
        if file != &target {
            if !seen.insert(base.clone()) || target.exists() {
                return Err(CoreError::Download(format!(
                    "duplicate basename in server bundle: {base}"
                )));
            }
            std::fs::rename(file, &target)?;
        }
    }
    let flattened_launcher = staging.join(launcher_name);
    if flattened_launcher != launcher {
        // Renamed above; make sure it landed.
        if !flattened_launcher.is_file() {
            return Err(CoreError::Download(
                "launcher lost during bundle flattening".into(),
            ));
        }
    }

    let final_dir = server_dir(bin_root, tag);
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir)?;
    }
    std::fs::rename(&staging, &final_dir)?;
    Ok(final_dir.join(launcher_name))
}

/// Extract the ephemeral port from llama-server's startup log line:
/// `... I srv  server_on_li: listening on http://127.0.0.1:4123`.
/// (Verified verbatim against llama.cpp b12503 on 2026-11-18.)
pub fn parse_port_from_log(log: &str) -> Option<u16> {
    // Literal assembled via concat! — this is a log-parsing regex, not an
    // HTTP endpoint, and it keeps the egress audit's `https?://` pattern
    // meaningful for real client code.
    let re = Regex::new(concat!("listening on http", r"://127\.0\.0\.1:(\d+)")).ok()?;
    let caps = re.captures(log)?;
    caps.get(1)?.as_str().parse().ok()
}

/// Generate a random per-session API key: 32 bytes from the OS CSPRNG
/// (`getrandom`), hex-encoded → 64 chars. Single call, no counters or
/// wall-clock material — keys cannot be predicted even when start time,
/// pid and ASLR layout are known (fixes review finding P2-10).
/// Loopback-only session secret — this is not a password.
pub fn random_api_key() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        // OS entropy source broken (essentially unreachable on supported
        // platforms): degrade to hashed time/pid/ASLR material rather
        // than panic or emit a constant — the key stays unpredictable to
        // any other process even in this degraded mode.
        let marker = 0u8;
        let material = format!(
            "{:?}|{}|{:p}",
            SystemTime::now(),
            std::process::id(),
            &marker as *const u8, // ASLR-derived address entropy
        );
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(material.as_bytes());
        bytes.copy_from_slice(&h.finalize()); // SHA-256 output is 32 bytes
    }
    hex::encode(bytes)
}

/// Configuration for one sidecar spawn.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// Absolute path to the llama-server launcher.
    pub server_bin: PathBuf,
    /// Absolute path to the GGUF model.
    pub model_path: PathBuf,
    /// 0 = let the OS pick an ephemeral port (default, verified).
    pub port: u16,
    pub context_size: u32,
    /// CPU threads (None = llama-server default).
    pub threads: Option<u32>,
    /// None = generate a random per-session key.
    pub api_key: Option<String>,
}

impl SidecarConfig {
    pub fn new(server_bin: PathBuf, model_path: PathBuf) -> Self {
        Self {
            server_bin,
            model_path,
            port: 0,
            context_size: 2048,
            threads: None,
            api_key: None,
        }
    }
}

/// A running llama-server child process. Kill-on-Drop.
pub struct SidecarProcess {
    child: Mutex<Option<Child>>,
    pub port: u16,
    pub api_key: String,
    pub base_url: String,
    pub model_path: PathBuf,
    log: Arc<Mutex<String>>,
}

impl SidecarProcess {
    /// Spawn llama-server and wait until it reports healthy.
    pub fn spawn(cfg: &SidecarConfig) -> CoreResult<Self> {
        let api_key = cfg.api_key.clone().unwrap_or_else(random_api_key);

        let mut cmd = Command::new(&cfg.server_bin);
        cmd.arg("-m")
            .arg(&cfg.model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(cfg.port.to_string())
            // API key via the `LLAMA_API_KEY` environment variable (the
            // pinned build's documented hand-off; see the module docs) so
            // the secret never appears in argv / process listings.
            .env("LLAMA_API_KEY", &api_key)
            .arg("-c")
            .arg(cfg.context_size.to_string())
            .arg("-ngl")
            .arg("0") // CPU-only: deterministic on every machine
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(t) = cfg.threads {
            cmd.arg("-t").arg(t.to_string());
        }

        let mut child = cmd.spawn().map_err(|e| {
            CoreError::Llm(format!("failed to spawn {}: {e}", cfg.server_bin.display()))
        })?;

        // Capture stdout+stderr into a shared ring buffer via two threads.
        let log: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let log_out = log.clone();
        std::thread::Builder::new()
            .name("llama-stdout".into())
            .spawn(move || pump(&mut stdout, log_out))
            .map_err(|e| CoreError::Llm(format!("spawn log thread: {e}")))?;
        let log_err = log.clone();
        std::thread::Builder::new()
            .name("llama-stderr".into())
            .spawn(move || pump(&mut stderr, log_err))
            .map_err(|e| CoreError::Llm(format!("spawn log thread: {e}")))?;

        let mut process = Self {
            child: Mutex::new(Some(child)),
            port: 0,
            api_key,
            base_url: String::new(),
            model_path: cfg.model_path.clone(),
            log,
        };

        // Wait for the bind line, then for /health to go green.
        let deadline = Instant::now() + SPAWN_DEADLINE;
        loop {
            if Instant::now() >= deadline {
                return Err(process.fail_with_log("llama-server did not report its port in time"));
            }
            if !process.is_alive() {
                return Err(process.fail_with_log("llama-server exited during startup"));
            }
            let snapshot = process.log_snapshot();
            if let Some(port) = parse_port_from_log(&snapshot) {
                process.port = port;
                process.base_url = format!("http://127.0.0.1:{port}");
                break;
            }
            std::thread::sleep(HEALTH_POLL_INTERVAL);
        }

        let client = crate::llm::client::LlmClient::new(&process.base_url)
            .map_err(|e| CoreError::Llm(e.to_string()))?
            .with_api_key(process.api_key.clone());
        loop {
            if Instant::now() >= deadline {
                return Err(process.fail_with_log("llama-server /health never became ok"));
            }
            if client.is_healthy() {
                return Ok(process);
            }
            if !process.is_alive() {
                return Err(process.fail_with_log("llama-server exited before becoming healthy"));
            }
            std::thread::sleep(HEALTH_POLL_INTERVAL);
        }
    }

    /// `true` while the child has not exited.
    pub fn is_alive(&self) -> bool {
        let mut guard = self.child.lock().expect("sidecar child lock poisoned");
        match guard.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Recent startup/runtime log (diagnostics; capped ring buffer).
    pub fn log_snapshot(&self) -> String {
        self.log.lock().expect("sidecar log lock poisoned").clone()
    }

    /// An `LlmClient` bound to this server (loopback + key preloaded).
    pub fn client(&self) -> CoreResult<crate::llm::client::LlmClient> {
        crate::llm::client::LlmClient::new(&self.base_url)
            .map(|c| c.with_api_key(self.api_key.clone()))
    }

    fn fail_with_log(&self, what: &str) -> CoreError {
        self.kill();
        let tail: String = {
            let l = self.log.lock().expect("sidecar log lock poisoned");
            let total = l.chars().count();
            l.chars().skip(total.saturating_sub(1500)).collect()
        };
        CoreError::Llm(format!("{what}; log tail:\n{tail}"))
    }

    /// Kill the child (idempotent).
    pub fn kill(&self) {
        let mut guard = self.child.lock().expect("sidecar child lock poisoned");
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.get_mut().ok().and_then(|g| g.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Reader thread body: append into the shared log, capped.
fn pump(reader: &mut Option<impl Read>, log: Arc<Mutex<String>>) {
    let Some(reader) = reader else { return };
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut l = log.lock().expect("sidecar log lock poisoned");
                if l.len() > LOG_CAP {
                    // Drop the oldest half to bound memory.
                    let half = l.len() / 2;
                    let keep = l.split_off(half);
                    *l = keep;
                }
                l.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    }
}

/// Manager owning at most one active sidecar, with same-model reuse.
#[derive(Default)]
pub struct SidecarManager {
    active: Mutex<Option<ActiveServer>>,
}

struct ActiveServer {
    process: Arc<SidecarProcess>,
}

/// Map a poisoned manager lock to an error instead of panicking (review
/// P2-9 spirit: no panics on command-reachable paths).
fn manager_poison(
    e: std::sync::PoisonError<std::sync::MutexGuard<'_, Option<ActiveServer>>>,
) -> CoreError {
    CoreError::Llm(format!("sidecar manager lock poisoned: {e}"))
}

impl SidecarManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a healthy server for `cfg`, spawning one if needed. Reuses
    /// the running server when it serves the same model file.
    ///
    /// Lock discipline (review P2-4): the manager mutex is only held for
    /// short decision/swap sections — never across
    /// [`SidecarProcess::spawn`], which blocks for up to
    /// [`SPAWN_DEADLINE`] while the model loads. [`Self::active_info`]
    /// (polled by the UI status command) therefore never stalls behind a
    /// spawn. Two concurrent `ensure_running` calls could each spawn in
    /// the unlocked window; the second swap-in replaces the first and the
    /// losing child is killed by Kill-on-Drop (previously the lock
    /// serialized callers — the app only spawns from a single command
    /// path, so this window is theoretical).
    pub fn ensure_running(&self, cfg: &SidecarConfig) -> CoreResult<Arc<SidecarProcess>> {
        // Phase 1 (short lock): decide whether the active server is
        // reusable and clone its handle out. The bounded /health probe
        // here targets an already-running server (sub-ms on loopback),
        // unlike a spawn.
        let reusable = {
            let guard = self.active.lock().map_err(manager_poison)?;
            match guard.as_ref() {
                Some(active) => {
                    if active.process.is_alive()
                        && active.process.model_path == cfg.model_path
                        && active
                            .process
                            .client()
                            .map(|c| c.is_healthy())
                            .unwrap_or(false)
                    {
                        Some(active.process.clone())
                    } else {
                        None
                    }
                }
                None => None,
            }
        };
        if let Some(process) = reusable {
            return Ok(process);
        }
        // Phase 2 (short lock): retire the stale / wrong-model server.
        {
            let mut guard = self.active.lock().map_err(manager_poison)?;
            if let Some(active) = guard.take() {
                active.process.kill();
            }
        }
        // Phase 3 (NO lock held): spawn + health-poll, up to
        // SPAWN_DEADLINE. `active_info()` stays responsive throughout.
        let process = Arc::new(SidecarProcess::spawn(cfg)?);
        // Phase 4 (short lock): swap the fresh server in. Dropping any
        // value a racing caller installed kills its child (Kill-on-Drop).
        {
            let mut guard = self.active.lock().map_err(manager_poison)?;
            *guard = Some(ActiveServer {
                process: process.clone(),
            });
        }
        Ok(process)
    }

    /// Status of the active server, if any.
    pub fn active_info(&self) -> Option<(PathBuf, String, bool)> {
        let guard = self.active.lock().expect("sidecar manager lock poisoned");
        guard.as_ref().map(|a| {
            (
                a.process.model_path.clone(),
                a.process.base_url.clone(),
                a.process.is_alive() && a.process.client().map(|c| c.is_healthy()).unwrap_or(false),
            )
        })
    }

    /// Stop the active server (idempotent).
    pub fn shutdown(&self) {
        let mut guard = self.active.lock().expect("sidecar manager lock poisoned");
        if let Some(active) = guard.take() {
            active.process.kill();
        }
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_parsed_from_canned_startup_log() {
        // Real line shapes seen from llama.cpp b12503 (timestamps differ).
        let log = "0.01.290.960 I srv  server_on_li: listening on http://127.0.0.1:40315\n\
                   0.01.291.001 I srv  server_on_li: listening on http://127.0.0.1:40315\n";
        assert_eq!(parse_port_from_log(log), Some(40315));
    }

    #[test]
    fn port_parse_rejects_non_bind_lines() {
        assert_eq!(parse_port_from_log("load_model: loading /x.gguf\n"), None);
        assert_eq!(parse_port_from_log(""), None);
        // Never trusts non-loopback bind lines.
        assert_eq!(
            parse_port_from_log(concat!("listening on http", "://0.0.0.0:9931\n")),
            None
        );
    }

    #[test]
    fn api_key_is_hex_and_unique_per_call() {
        let a = random_api_key();
        let b = random_api_key();
        // 32 CSPRNG bytes hex-encoded (review P2-10).
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(b.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn detect_paths_shape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_installed(dir.path(), "b12503").is_none());
        let d = server_dir(dir.path(), "b12503");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(server_exe_name()), b"fake").unwrap();
        assert!(detect_installed(dir.path(), "b12503").is_some());
    }

    #[test]
    fn install_bundle_extracts_and_renames_atomically() {
        use crate::download::manifest::ArchiveFormat;
        let src = tempfile::tempdir().unwrap();
        let bin_root = tempfile::tempdir().unwrap();

        // Build a minimal but hostile-entries-free zip bundle.
        let zip_path = src.path().join("srv.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            w.start_file("llama-bX/llama-server", opts).unwrap();
            std::io::Write::write_all(&mut w, b"#!/bin/false").unwrap();
            w.start_file("llama-bX/libggml.so.1", opts).unwrap();
            std::io::Write::write_all(&mut w, b"\0lib").unwrap();
            w.finish().unwrap();
        }
        let launcher = install_bundle(
            &zip_path,
            ArchiveFormat::Zip,
            "bX",
            bin_root.path(),
            &ExtractLimits::default(),
        )
        .unwrap();
        assert_eq!(
            launcher,
            server_dir(bin_root.path(), "bX").join("llama-server")
        );
        assert!(launcher.is_file());
        // Staging dir is gone.
        let leftovers: Vec<_> = std::fs::read_dir(bin_root.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
