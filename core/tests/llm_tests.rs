//! Integration tests for the LLM layer.
//!
//! - Deterministic tests drive the SSE client against a per-test
//!   127.0.0.1 mock server — no external network, no llama-server.
//! - `llm_real_roundtrip` (`#[ignore]`) downloads the pinned llama-server
//!   bundle + the pinned test GGUF **through the download manager** into a
//!   cache dir, spawns the real sidecar and runs one real explanation
//!   roundtrip. CI runs it (llm-smoke job); locally:
//!   `cargo test -p difflens-core -- --ignored --nocapture llm_real_roundtrip`
//!   Set `DIFFLENS_SKIP_LLM_SMOKE=1` to skip with a message.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use difflens_core::download::checksum;
use difflens_core::download::manifest::LlmManifest;
use difflens_core::download::{DownloadSpec, download_file};
use difflens_core::llm::client::{ChatMessage, ChatOptions, LlmClient};
use difflens_core::llm::explain::{
    ContextBudget, Lang, build_explanation_messages, change_context_from, parse_explanation,
};
use difflens_core::llm::sidecar::{SidecarConfig, SidecarProcess};

// ---------------------------------------------------------------------------
// Deterministic: mock SSE server
// ---------------------------------------------------------------------------

/// Tiny loopback HTTP server that captures the request body and streams
/// canned SSE frames.
struct MockSseServer {
    addr: String,
    captured_request: std::sync::Arc<std::sync::Mutex<String>>,
}

impl MockSseServer {
    fn spawn(frames: &[&str], delay_first: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let captured_t = captured.clone();
        let frames: Vec<String> = frames.iter().map(|s| s.to_string()).collect();
        std::thread::spawn(move || {
            let (mut stream, _) = match listener.accept() {
                Ok(x) => x,
                Err(_) => return,
            };
            // Read headers + body until Content-Length satisfied.
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(idx) = text.find("\r\n\r\n") {
                    let content_len = text
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if raw.len() >= idx + 4 + content_len {
                        break;
                    }
                }
            }
            *captured_t.lock().unwrap() = String::from_utf8_lossy(&raw).to_string();
            if delay_first {
                std::thread::sleep(Duration::from_millis(150));
            }
            let mut payload = String::new();
            for f in &frames {
                payload.push_str(f);
                payload.push_str("\n\n");
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(payload.as_bytes()).ok();
        });
        Self {
            addr,
            captured_request: captured,
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

#[test]
fn chat_stream_roundtrip_against_mock_sse() {
    let server = MockSseServer::spawn(
        &[
            "data: {\"choices\":[{\"delta\":{\"content\":\"Deadline \"}}]}",
            "data: {\"choices\":[{\"delta\":{\"content\":\"moved.\"}}]}",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}",
            "data: [DONE]",
        ],
        false,
    );
    let client = LlmClient::new(&server.url()).unwrap();
    let opts = ChatOptions::default();
    let mut tokens: Vec<String> = Vec::new();
    let result = client
        .chat_stream(
            &[ChatMessage::user("explain c0")],
            &opts,
            &mut |tok: &str| {
                tokens.push(tok.to_string());
                true
            },
        )
        .unwrap();
    assert_eq!(result.text, "Deadline moved.");
    assert_eq!(tokens, vec!["Deadline ".to_string(), "moved.".to_string()]);
    assert_eq!(result.finish_reason.as_deref(), Some("stop"));

    // The wire request was a streaming chat completion.
    let req = server.captured_request.lock().unwrap().clone();
    assert!(req.contains("POST /v1/chat/completions"));
    assert!(req.contains("\"stream\":true") || req.contains("\"stream\": true"));
}

#[test]
fn chat_stream_cancellation_returns_cancelled_error() {
    let server = MockSseServer::spawn(
        &[
            "data: {\"choices\":[{\"delta\":{\"content\":\"tok\"}}]}",
            "data: [DONE]",
        ],
        true,
    );
    let client = LlmClient::new(&server.url()).unwrap();
    let err = client
        .chat_stream(
            &[ChatMessage::user("x")],
            &ChatOptions::default(),
            &mut |_tok| false, // cancel on first token
        )
        .unwrap_err();
    assert!(
        matches!(err, difflens_core::CoreError::Cancelled),
        "expected Cancelled, got {err}"
    );
}

#[test]
fn chat_stream_survives_malformed_frames() {
    let server = MockSseServer::spawn(
        &[
            "data: garbage-not-json",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}",
            "data: [DONE]",
        ],
        false,
    );
    let client = LlmClient::new(&server.url()).unwrap();
    let result = client
        .chat_stream(&[], &ChatOptions::default(), &mut |_| true)
        .unwrap();
    assert_eq!(result.text, "ok");
}

#[test]
fn client_rejects_non_loopback_before_any_connection() {
    assert!(LlmClient::new("http://evil.example.com:8080").is_err());
    assert!(LlmClient::new("http://10.0.0.5").is_err());
    // A rejected client never produced a base_url to dial.
    let err = match LlmClient::new("http://evil.example.com") {
        Err(e) => e,
        Ok(_) => panic!("non-loopback client was accepted"),
    };
    assert!(err.to_string().contains("non-loopback"));
}

#[test]
fn prompt_end_to_end_into_mock_stream() {
    // Full deterministic path: Change -> prompt -> (mock) tokens -> parse.
    use difflens_core::model::{Change, ChangeKind, DiffSpan, HeuristicFlag, Severity, SpanOp};
    let change = Change {
        id: "c1".into(),
        kind: ChangeKind::Modified,
        old_block_ids: vec!["b0".into()],
        new_block_ids: vec!["b0".into()],
        old_page: Some(1),
        new_page: Some(1),
        spans: vec![
            DiffSpan {
                op: SpanOp::Equal,
                text: "Penalty is ".into(),
            },
            DiffSpan {
                op: SpanOp::Delete,
                text: "5%".into(),
            },
            DiffSpan {
                op: SpanOp::Insert,
                text: "10%".into(),
            },
        ],
        old_text: None,
        new_text: None,
        similarity: Some(0.9),
        move_delta: None,
        severity: Some(Severity::Critical),
        flags: vec![HeuristicFlag {
            rule_id: "PERCENT_CHANGED".into(),
            label_key: "flag.percentChanged".into(),
            old_value: Some("5%".into()),
            new_value: Some("10%".into()),
            delta: Some(1.0),
            detail: None,
        }],
    };
    let ctx = change_context_from(&change);
    assert_eq!(ctx.old_text, "Penalty is 5%");
    assert_eq!(ctx.new_text, "Penalty is 10%");
    let msgs = build_explanation_messages(&ctx, Lang::En, &ContextBudget::default());
    assert!(msgs[1].content.contains("c1"));
    assert!(msgs[1].content.contains("PERCENT_CHANGED"));

    // Model "reply" (canned): fenced JSON — parse tolerantly.
    let parsed = parse_explanation(
        "```json\n{\"summary\":\"Penalty doubled from 5% to 10%.\",\"impact\":\"Breach costs double.\"}\n```",
    );
    assert_eq!(parsed.summary, "Penalty doubled from 5% to 10%.");
    assert_eq!(parsed.impact, "Breach costs double.");
}

// ---------------------------------------------------------------------------
// Real smoke test (ignored; runs in CI `llm-smoke` job)
// ---------------------------------------------------------------------------

/// Cache root for the real test. Overridable for local runs; defaults to
/// `target/llm-cache` (gitignored).
fn llm_cache_root() -> PathBuf {
    if let Some(p) = std::env::var_os("DIFFLENS_LLM_CACHE") {
        return PathBuf::from(p);
    }
    // CARGO_MANIFEST_DIR = core/; target/ lives at the workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("llm-cache")
}

#[test]
#[ignore = "real network + real llama-server roundtrip; run with: cargo test -p difflens-core -- --ignored llm_real_roundtrip"]
fn llm_real_roundtrip() {
    if std::env::var("DIFFLENS_SKIP_LLM_SMOKE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        eprintln!("SKIP: DIFFLENS_SKIP_LLM_SMOKE=1");
        return;
    }
    let manifest = LlmManifest::embedded();
    let Some(asset) = manifest.server_asset_for_current_os() else {
        eprintln!(
            "SKIP: no pinned llama-server asset for {} in the manifest",
            difflens_core::download::manifest::current_target_triple()
        );
        return;
    };
    let Some(model) = manifest.model("qwen3-0.6b-q8_0-test") else {
        panic!("manifest lost its test model entry");
    };

    let cache = llm_cache_root();
    let bin_root = cache.join("bin");
    let models_dir = cache.join("models");
    std::fs::create_dir_all(&bin_root).unwrap();
    std::fs::create_dir_all(&models_dir).unwrap();

    // 1. Download (resume-capable) + verify the server bundle.
    let archive_path = bin_root.join(format!(
        "llama-{}-bin-{}.archive",
        manifest.llama_server.tag, asset.id
    ));
    if !archive_path.exists() {
        let spec = DownloadSpec::new(
            asset.url.clone(),
            archive_path.clone(),
            asset.sha256.clone(),
        );
        let t0 = Instant::now();
        let mut spec = spec;
        spec.expected_bytes = Some(asset.bytes);
        download_file(&spec, &mut |done, total| {
            eprintln!(
                "  server bundle: {} / {} bytes",
                done,
                total.map(|t| t.to_string()).unwrap_or_else(|| "?".into())
            );
            true
        })
        .unwrap();
        eprintln!("  server bundle downloaded in {:?}", t0.elapsed());
    }

    // 2. Install (hardened extraction) unless already installed.
    let server_bin = match difflens_core::llm::sidecar::detect_installed(
        &bin_root,
        &manifest.llama_server.tag,
    ) {
        Some(p) => p,
        None => difflens_core::llm::sidecar::install_bundle(
            &archive_path,
            asset.archive,
            &manifest.llama_server.tag,
            &bin_root,
            &difflens_core::download::archive::ExtractLimits::default(),
        )
        .unwrap(),
    };
    assert!(server_bin.is_file(), "llama-server launcher missing");

    // 3. Download + verify the test GGUF (639 MB, sha256-pinned).
    let model_path = models_dir.join(&model.file);
    if !model_path.exists() {
        let mut spec =
            DownloadSpec::new(model.url.clone(), model_path.clone(), model.sha256.clone());
        spec.expected_bytes = Some(model.bytes);
        let t0 = Instant::now();
        let mut last_report = Instant::now();
        download_file(&spec, &mut |done, total| {
            if last_report.elapsed() >= Duration::from_secs(5) {
                last_report = Instant::now();
                eprintln!(
                    "  model: {} / {} bytes",
                    done,
                    total.map(|t| t.to_string()).unwrap_or_else(|| "?".into())
                );
            }
            true
        })
        .unwrap();
        eprintln!("  model downloaded in {:?}", t0.elapsed());
    }
    // Re-verify checksum of cached model (cache could be corrupt).
    checksum::verify_file(&model_path, &model.sha256, 0).unwrap();

    // 4. Spawn the real sidecar (loopback, random key, CPU-only, small ctx).
    let mut cfg = SidecarConfig::new(server_bin.clone(), model_path);
    cfg.context_size = 2048;
    cfg.threads = Some(2); // 3-core VM: leave a core for the test process
    let t0 = Instant::now();
    let server = SidecarProcess::spawn(&cfg).unwrap();
    eprintln!(
        "  sidecar ready in {:?} at {} (key hidden)",
        t0.elapsed(),
        server.base_url
    );

    // 5. One real explanation roundtrip (EN).
    use difflens_core::model::{Change, ChangeKind, DiffSpan, HeuristicFlag, Severity, SpanOp};
    let change = Change {
        id: "c0".into(),
        kind: ChangeKind::Modified,
        old_block_ids: vec![],
        new_block_ids: vec![],
        old_page: Some(1),
        new_page: Some(1),
        spans: vec![
            DiffSpan {
                op: SpanOp::Equal,
                text: "Payment within ".into(),
            },
            DiffSpan {
                op: SpanOp::Delete,
                text: "30 days".into(),
            },
            DiffSpan {
                op: SpanOp::Insert,
                text: "45 days".into(),
            },
        ],
        similarity: Some(0.9),
        move_delta: None,
        old_text: None,
        new_text: None,
        severity: Some(Severity::Medium),
        flags: vec![HeuristicFlag {
            rule_id: "MONEY_CHANGED".into(),
            label_key: "flag.moneyChanged".into(),
            old_value: Some("30 days".into()),
            new_value: Some("45 days".into()),
            delta: Some(0.5),
            detail: None,
        }],
    };
    let ctx = change_context_from(&change);
    let messages = build_explanation_messages(&ctx, Lang::En, &ContextBudget::default());

    let client = server.client().unwrap();
    let opts = ChatOptions {
        model: "test".into(),
        max_tokens: 220,
        ..ChatOptions::default()
    };
    let t1 = Instant::now();
    let mut token_count = 0usize;
    let result = client
        .chat_stream(&messages, &opts, &mut |_tok| {
            token_count += 1;
            true
        })
        .expect("real chat stream failed");
    let elapsed = t1.elapsed();
    eprintln!(
        "  roundtrip: {token_count} tokens in {:?} ({:.1} tok/s), finish_reason={:?}",
        elapsed,
        token_count as f64 / elapsed.as_secs_f64().max(0.001),
        result.finish_reason
    );

    // Assertions: non-empty explanation, valid SSE termination, parses.
    assert!(
        !result.text.trim().is_empty(),
        "model returned an empty explanation"
    );
    assert!(
        matches!(
            result.finish_reason.as_deref(),
            Some("stop") | Some("length")
        ),
        "stream did not terminate cleanly: {:?}",
        result.finish_reason
    );
    let parsed = parse_explanation(&result.text);
    eprintln!("  summary: {}", parsed.summary);
    assert!(!parsed.summary.trim().is_empty());

    // 6. And a VI roundtrip — same server, second language prompt.
    let messages_vi = build_explanation_messages(&ctx, Lang::Vi, &ContextBudget::default());
    let result_vi = client
        .chat_stream(
            &messages_vi,
            &ChatOptions {
                model: "test".into(),
                max_tokens: 220,
                ..ChatOptions::default()
            },
            &mut |_| true,
        )
        .expect("VI chat stream failed");
    assert!(!result_vi.text.trim().is_empty(), "VI explanation empty");
    eprintln!(
        "  vi: {}",
        result_vi.text.chars().take(120).collect::<String>()
    );

    // Clean shutdown (Drop would also kill it).
    server.kill();
}
