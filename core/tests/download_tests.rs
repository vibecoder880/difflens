//! Integration tests for the download manager (the workspace's only
//! network egress module). All network here is a per-test 127.0.0.1
//! loopback server — fully deterministic, no external traffic.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use difflens_core::download::checksum;
use difflens_core::download::manifest::{ArchiveFormat, LlmManifest};
use difflens_core::download::{DownloadManager, DownloadSpec};

/// Loopback HTTP server serving a fixed body with optional Range support.
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
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
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
                        let head =
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n";
                        stream.write_all(head.as_bytes()).ok();
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

fn wait_terminal(mgr: &DownloadManager, job: &str) -> difflens_core::download::JobSnapshot {
    for _ in 0..300 {
        let snap = mgr.snapshot(job).unwrap();
        if snap.state != difflens_core::download::JobState::Running {
            return snap;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("download job never reached a terminal state");
}

fn zip_bundle(entries: &[(&str, &[u8])], dir: &Path) -> PathBuf {
    let path = dir.join("bundle.zip");
    let f = std::fs::File::create(&path).unwrap();
    let mut w = zip::ZipWriter::new(f);
    for (name, data) in entries {
        let opts: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        w.start_file(*name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap();
    path
}

#[test]
fn manifest_embedded_loads_with_pinned_provenance() {
    let m = LlmManifest::embedded();
    assert!(m.llama_server.tag.starts_with('b'));
    assert!(m.server_asset_for_current_os().is_some());
    for asset in &m.llama_server.assets {
        assert!(asset.url.starts_with("https://github.com/"));
        assert_eq!(asset.sha256.len(), 64);
        assert!(asset.verification.contains("2026-11-18"));
    }
    for model in &m.models {
        assert!(model.url.starts_with("https://huggingface.co/"));
        assert_eq!(model.sha256.len(), 64);
        assert!(model.license.contains("Apache"));
    }
}

#[test]
fn manager_full_flow_with_checksum_and_progress() {
    let body: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    let sha = checksum::sha256_bytes(&body);
    let server = LoopbackServer::spawn(body.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("artifact.bin");

    let mut spec = DownloadSpec::new(
        format!("http://{}/a", server.addr),
        dest.clone(),
        sha.clone(),
    );
    spec.expected_bytes = Some(body.len() as u64);

    let mgr = DownloadManager::new();
    let max_seen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let max_seen_t = max_seen.clone();
    let job = mgr.start(spec, move |done, _| {
        max_seen_t.fetch_max(done, std::sync::atomic::Ordering::SeqCst);
    });
    let snap = wait_terminal(&mgr, &job);
    assert_eq!(snap.state, difflens_core::download::JobState::Done);
    assert_eq!(snap.bytes_done, body.len() as u64);
    assert_eq!(snap.bytes_total, Some(body.len() as u64));
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert!(
        max_seen.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "progress callback was never invoked"
    );
    assert!(!dest.with_file_name("artifact.bin.part").exists());
    // Cancelling a finished job is a no-op.
    assert!(!mgr.cancel(&job));
}

#[test]
fn manager_resume_across_restart_keeps_part() {
    let body = b"abcdef0123456789".to_vec();
    let sha = checksum::sha256_bytes(&body);
    let server = LoopbackServer::spawn(body.clone(), true);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("m.bin");

    // First attempt: cancel immediately (part may or may not exist yet).
    let mut spec = DownloadSpec::new(
        format!("http://{}/m", server.addr),
        dest.clone(),
        sha.clone(),
    );
    spec.expected_bytes = Some(body.len() as u64);
    let mgr = DownloadManager::new();
    let job = mgr.start(spec.clone(), |_, _| {});
    let _ = mgr.cancel(&job);
    let snap = wait_terminal(&mgr, &job);
    assert_eq!(snap.state, difflens_core::download::JobState::Cancelled);

    // Second attempt resumes (or restarts) and completes.
    let job2 = mgr.start(spec, |_, _| {});
    let snap2 = wait_terminal(&mgr, &job2);
    assert_eq!(snap2.state, difflens_core::download::JobState::Done);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
}

#[test]
fn manager_checksum_failure_marks_job_failed() {
    let body = b"payload".to_vec();
    let server = LoopbackServer::spawn(body, false);
    let dir = tempfile::tempdir().unwrap();
    let mut spec = DownloadSpec::new(
        format!("http://{}/x", server.addr),
        dir.path().join("x.bin"),
        checksum::sha256_bytes(b"different"),
    );
    spec.expected_bytes = Some(7);
    let mgr = DownloadManager::new();
    let job = mgr.start(spec, |_, _| {});
    let snap = wait_terminal(&mgr, &job);
    assert_eq!(snap.state, difflens_core::download::JobState::Failed);
    assert!(snap.error.unwrap_or_default().contains("checksum mismatch"));
}

#[test]
fn install_bundle_then_detect_roundtrip() {
    let src = tempfile::tempdir().unwrap();
    let bin_root = tempfile::tempdir().unwrap();
    let archive = zip_bundle(
        &[
            ("llama-bT/llama-server", b"#!/bin/false"),
            ("llama-bT/libggml.so.1", b"\0lib"),
        ],
        src.path(),
    );
    let launcher = difflens_core::llm::sidecar::install_bundle(
        &archive,
        ArchiveFormat::Zip,
        "bT",
        bin_root.path(),
        &difflens_core::download::archive::ExtractLimits::default(),
    )
    .unwrap();
    assert_eq!(
        launcher,
        difflens_core::llm::sidecar::server_dir(bin_root.path(), "bT").join("llama-server")
    );
    assert!(launcher.is_file());
    assert!(difflens_core::llm::sidecar::detect_installed(bin_root.path(), "bT").is_some());
    assert!(difflens_core::llm::sidecar::detect_installed(bin_root.path(), "bOther").is_none());
}
