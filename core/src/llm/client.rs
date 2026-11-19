//! OpenAI-compatible streaming chat client for the local llama-server
//! sidecar (`POST /v1/chat/completions`, SSE).
//!
//! Security invariants (red-team SEC3):
//! - **Loopback-only**: [`LlmClient::new`] hard-rejects any base URL whose
//!   host is not the literal `127.0.0.1` / `::1` (`[::1]`) — DNS names
//!   (`localhost` included) are rejected because their resolution depends
//!   on the host resolver, which a hostile resolver could point elsewhere
//!   while the client attaches `Authorization: Bearer <session key>`.
//!   The sidecar must never be reachable from outside the machine, and
//!   the client must never point anywhere else. Unit-tested.
//! - **API key in memory only**: the optional bearer token lives inside
//!   this struct for the process lifetime, is never persisted, and
//!   defaults to none (local server without a key).
//! - **Model output is untrusted text** (SEC4): tokens are forwarded as
//!   plain strings; this layer does no HTML/markdown processing.

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::error::{CoreError, CoreResult};

/// Default overall request timeout (prompt processing on CPU can be slow;
/// still a hard bound, unlike no timeout at all).
pub const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Hard cap for the assistant text accumulated from one streamed
/// completion. A misbehaving (or compromised) server can ignore
/// `max_tokens`; without this bound every token grows the buffer until
/// the request timeout. Well beyond anything the 512-token explanations
/// produce, yet bounded (~256 KiB per request).
pub const MAX_EXPLANATION_BYTES: usize = 256 * 1024;

/// One chat message.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

/// Per-call options.
#[derive(Debug, Clone)]
pub struct ChatOptions {
    /// Model name/path — llama-server accepts anything; it serves the
    /// model it was started with.
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Bearer token; `None` for a keyless local server.
    pub api_key: Option<String>,
    /// Suppress Qwen3 hybrid thinking mode (`enable_thinking=false`).
    pub disable_thinking: bool,
    /// Hard cap for the entire request including body streaming.
    pub overall_timeout: Duration,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            model: "default".into(),
            temperature: 0.0,
            max_tokens: 512,
            api_key: None,
            disable_thinking: true,
            overall_timeout: DEFAULT_OVERALL_TIMEOUT,
        }
    }
}

/// Outcome of a streamed completion.
#[derive(Debug, Clone)]
pub struct StreamResult {
    /// The full assistant text (concatenated deltas).
    pub text: String,
    /// OpenAI finish reason from the final chunk, when sent.
    pub finish_reason: Option<String>,
    /// Number of SSE `data:` frames parsed (diagnostics).
    pub frames: usize,
}

/// Loopback-guarded streaming client.
pub struct LlmClient {
    base_url: String,
    /// Optional bearer key (SEC3: in-memory only, never persisted).
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl LlmClient {
    /// Construct a client for a **loopback** base URL (e.g.
    /// `http://127.0.0.1:4123`). Any other host is a hard error.
    pub fn new(base_url: &str) -> CoreResult<Self> {
        validate_loopback(base_url)?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: None,
            agent: crate::download::build_llm_agent(),
        })
    }

    /// Attach the per-session bearer key (llama-server `--api-key`).
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// The validated base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `true` when `/health` reports the server ready (best effort —
    /// network errors map to `false`, never panic).
    pub fn is_healthy(&self) -> bool {
        let mut req = self
            .agent
            .get(&format!("{}/health", self.base_url))
            .timeout(Duration::from_secs(5));
        if let Some(key) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        matches!(req.call(), Ok(resp) if resp.status() == 200)
    }

    /// POST `/v1/chat/completions` with `stream: true` and forward every
    /// content delta to `on_token`. The callback returns `false` to
    /// cancel (the connection is dropped, [`CoreError::Cancelled`] is
    /// returned, the partial text is lost — the sidecar keeps running).
    ///
    /// Malformed SSE frames are skipped, never fatal and never panicking.
    pub fn chat_stream(
        &self,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        on_token: &mut dyn FnMut(&str) -> bool,
    ) -> CoreResult<StreamResult> {
        validate_loopback(&self.base_url)?; // re-check on every call
        let mut body = json!({
            "model": opts.model,
            "messages": messages,
            "stream": true,
            "temperature": opts.temperature,
            "max_tokens": opts.max_tokens,
            "cache_prompt": true,
        });
        if opts.disable_thinking {
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        }

        let mut req = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base_url))
            .timeout(opts.overall_timeout)
            .set("Content-Type", "application/json");
        // Per-call key wins over the client-level key.
        let key = opts.api_key.as_ref().or(self.api_key.as_ref());
        if let Some(key) = key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        let resp = req
            .send_json(body)
            .map_err(|e| CoreError::Llm(format!("chat request failed: {e}")))?;
        if resp.status() != 200 {
            return Err(CoreError::Llm(format!(
                "chat endpoint returned HTTP {}",
                resp.status()
            )));
        }

        let mut stream = SseParser::new();
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 8 * 1024];
        let mut text = String::new();
        let mut finish_reason: Option<String> = None;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| CoreError::Llm(format!("sse read: {e}")))?;
            if n == 0 {
                break;
            }
            for event in stream.feed(&buf[..n]) {
                match event {
                    SseEvent::Token(tok) => {
                        if !on_token(&tok) {
                            return Err(CoreError::Cancelled);
                        }
                        if text.len() + tok.len() > MAX_EXPLANATION_BYTES {
                            // Checked before pushing: accumulated text
                            // never exceeds the cap plus one token.
                            return Err(CoreError::Llm("explanation exceeded size limit".into()));
                        }
                        text.push_str(&tok);
                    }
                    SseEvent::Finished(reason) => finish_reason = reason,
                    SseEvent::ServerError(msg) => {
                        return Err(CoreError::Llm(format!("server error frame: {msg}")));
                    }
                    SseEvent::Done => {
                        return Ok(StreamResult {
                            text,
                            finish_reason,
                            frames: stream.frames,
                        });
                    }
                    SseEvent::Skipped => {}
                }
            }
        }
        // Stream ended without [DONE]; treat as complete with what we have.
        Ok(StreamResult {
            text,
            finish_reason,
            frames: stream.frames,
        })
    }
}

/// Reject any base URL that is not loopback. Accepts `http://` and
/// `https://` schemes; host must be the **literal** `127.0.0.1` or
/// `[::1]` / `::1`. DNS names — `localhost` included — are deliberately
/// rejected: resolution depends on the host resolver, and a hostile
/// resolver could answer `localhost` (or any name we might later accept)
/// with a non-loopback address while the client sends the session bearer
/// key. Note `0.0.0.0` is also rejected (it is an exposed interface, not
/// a loopback).
pub fn validate_loopback(base_url: &str) -> CoreResult<()> {
    let url = base_url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some(pair) => pair,
        None => {
            return Err(CoreError::Llm(format!(
                "base url must include a scheme: {base_url:?}"
            )));
        }
    };
    if scheme != "http" && scheme != "https" {
        return Err(CoreError::Llm(format!(
            "base url scheme must be http(s): {scheme:?}"
        )));
    }
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = if host_port.starts_with('[') {
        // IPv6 literal: take up to and including `]`.
        match host_port.find(']') {
            Some(end) => &host_port[..=end],
            None => host_port,
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host_port,
        }
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    // Literal addresses only — no DNS names (see the doc comment above).
    if matches!(host.as_str(), "127.0.0.1" | "::1") {
        return Ok(());
    }
    Err(CoreError::Llm(format!(
        "non-loopback base url rejected: {base_url:?} (llama-server must stay on 127.0.0.1)"
    )))
}

/// SSE events emitted by the incremental parser.
#[derive(Debug, Clone, PartialEq)]
pub enum SseEvent {
    /// A content delta.
    Token(String),
    /// Terminal `[DONE]` sentinel.
    Done,
    /// A chunk carried a finish reason (not terminal by itself).
    Finished(Option<String>),
    /// llama.cpp error frame.
    ServerError(String),
    /// A frame we parsed but that carries no content.
    Skipped,
}

/// Incremental SSE parser: feed raw bytes (chunk boundaries may split
/// tokens mid-UTF-8 or mid-line), get out complete events. Stateless
/// carry-over between `feed` calls; `[DONE]` may be repeated safely (only
/// the first yields `Done`... actually every occurrence yields `Done`; the
/// caller returns on the first).
#[derive(Debug, Default)]
pub struct SseParser {
    carry: Vec<u8>,
    pub frames: usize,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes; returns events for every complete line processed.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.carry.extend_from_slice(bytes);
        let mut events = Vec::new();
        // Process only complete lines; keep the tail in `carry`.
        while let Some(pos) = self.carry.iter().position(|&b| b == b'\n') {
            let mut line_bytes: Vec<u8> = self.carry.drain(..=pos).collect();
            line_bytes.pop(); // drop the '\n'
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            let line = String::from_utf8_lossy(&line_bytes);
            if let Some(ev) = self.handle_line(&line) {
                events.push(ev);
            }
        }
        // Guard against pathological unbounded carry (no newlines ever).
        if self.carry.len() > 1024 * 1024 {
            self.carry.clear();
        }
        events
    }

    fn handle_line(&mut self, line: &str) -> Option<SseEvent> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            return None; // keep-alive / comment
        }
        let payload = trimmed.strip_prefix("data:")?.trim_start();
        self.frames += 1;
        if payload == "[DONE]" {
            return Some(SseEvent::Done);
        }
        // llama.cpp may send raw JSON error frames without `data:` — but
        // if it does arrive as data, surface it.
        match serde_json::from_str::<Chunk>(payload) {
            Ok(chunk) => {
                if let Some(err) = chunk.error {
                    return Some(SseEvent::ServerError(err_message(&err)));
                }
                let Some(choice) = chunk.choices.into_iter().next() else {
                    return Some(SseEvent::Skipped);
                };
                if let Some(reason) = choice.finish_reason {
                    return Some(SseEvent::Finished(Some(reason)));
                }
                match choice.delta.and_then(|d| d.content) {
                    Some(tok) if !tok.is_empty() => Some(SseEvent::Token(tok)),
                    _ => Some(SseEvent::Skipped),
                }
            }
            Err(_) => {
                // Malformed frame: skip silently (robustness over panic).
                tracing::debug!(target: "difflens::llm", "skipping malformed SSE frame: {payload:.120}");
                Some(SseEvent::Skipped)
            }
        }
    }
}

fn err_message(err: &serde_json::Value) -> String {
    match err {
        serde_json::Value::String(s) => s.clone(),
        v => v
            .get("message")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| v.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_guard_accepts_literal_loopback_hosts() {
        for url in [
            "http://127.0.0.1:8080",
            "http://[::1]:9931",
            "http://[::1]",
            "https://127.0.0.1:8080",
            "http://127.0.0.1/v1",
        ] {
            assert!(validate_loopback(url).is_ok(), "should accept {url}");
        }
    }

    #[test]
    fn loopback_guard_rejects_remote_dns_and_garbage() {
        for url in [
            // DNS names are rejected outright — `localhost` resolution
            // depends on the host resolver and could be poisoned (P2-2).
            "http://localhost:1234",
            "http://localhost",
            "http://LOCALHOST:80",
            "http://example.com",
            "https://api.openai.com/v1",
            "http://0.0.0.0:9931",
            "http://192.168.1.10:8080",
            "http://127.0.0.2:8080", // not the loopback host we allow
            "ftp://127.0.0.1",
            "127.0.0.1:8080", // no scheme
            "",
            "http://",
        ] {
            assert!(validate_loopback(url).is_err(), "should reject {url}");
        }
    }

    #[test]
    fn client_constructor_rejects_remote() {
        assert!(LlmClient::new("http://example.com").is_err());
        assert!(LlmClient::new("http://127.0.0.1:1").is_ok());
    }

    #[test]
    fn sse_parses_stream_including_split_tokens() {
        let raw = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo w\"}}]}\n\n",
            "data: {\"choi", // frame split mid-JSON-key across feeds
        );
        let mut p = SseParser::new();
        let mut evs = p.feed(raw.as_bytes());
        evs.extend(p.feed("ces\":[{\"delta\":{\"content\":\"orld\"}}]}\n\n".as_bytes()));
        evs.extend(p.feed("data: [DONE]\n\n".as_bytes()));

        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                SseEvent::Token(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello world");
        assert!(evs.contains(&SseEvent::Done));
    }

    #[test]
    fn sse_survives_split_multibyte_utf8() {
        // "é" = 0xC3 0xA9 split across chunks.
        let mut p = SseParser::new();
        let mut all = p.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"caf");
        all.extend(p.feed(&[0xC3]));
        all.extend(p.feed(&[0xA9, b'"', b'}', b'}', b']', b'}', b'\n', b'\n']));
        assert_eq!(all, vec![SseEvent::Token("café".to_string())]);
    }

    #[test]
    fn sse_skips_malformed_frames_without_panic() {
        let mut p = SseParser::new();
        let evs = p.feed(
            concat!(
                "data: not-json-at-all\n\n",
                "data: {\"broken\": \n\n",
                ": comment\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            )
            .as_bytes(),
        );
        assert_eq!(
            evs,
            vec![
                SseEvent::Skipped,
                SseEvent::Skipped,
                SseEvent::Token("ok".to_string())
            ]
        );
    }

    #[test]
    fn sse_surfaces_server_error_frames() {
        let mut p = SseParser::new();
        let evs = p.feed(b"data: {\"error\":{\"message\":\"model failed\"}}\n\n");
        assert_eq!(evs, vec![SseEvent::ServerError("model failed".to_string())]);
    }

    #[test]
    fn sse_captures_finish_reason_then_done() {
        let mut p = SseParser::new();
        let evs = p.feed(
            concat!(
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            )
            .as_bytes(),
        );
        assert_eq!(
            evs,
            vec![SseEvent::Finished(Some("stop".to_string())), SseEvent::Done]
        );
    }

    /// Fake SSE server: keeps streaming tokens (well past the 256 KiB
    /// cap) until the client disconnects. Loopback-only, no [DONE] ever.
    struct RunawaySseServer {
        addr: String,
    }

    impl RunawaySseServer {
        fn spawn() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            std::thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let head = concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n"
                );
                use std::io::Write;
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                let frame = concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"",
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "\"}}]}\n\n"
                );
                for _ in 0..100_000 {
                    // Breaks out once the client drops the connection
                    // (the cap error path under test).
                    if stream.write_all(frame.as_bytes()).is_err() {
                        break;
                    }
                }
            });
            Self { addr }
        }
    }

    #[test]
    fn chat_stream_errors_when_text_exceeds_cap() {
        let server = RunawaySseServer::spawn();
        let client = LlmClient::new(&format!("http://{}", server.addr)).unwrap();
        let opts = ChatOptions::default();
        let mut delivered = 0usize;
        let err = client
            .chat_stream(&[ChatMessage::user("hi")], &opts, &mut |_| {
                delivered += 1;
                true
            })
            .unwrap_err();
        assert!(
            matches!(err, CoreError::Llm(ref m) if m.contains("explanation exceeded size limit")),
            "unexpected error: {err}"
        );
        // The stream was live (tokens flowed) before the cap tripped.
        assert!(delivered > 0);
        // The cap (plus one in-flight token) bounds the buffer — nothing
        // here can hold 100k frames' worth of text.
    }
}
