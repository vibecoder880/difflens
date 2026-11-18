//! LLM sidecar module: local llama-server process management, the
//! loopback-only streaming chat client and deterministic prompt builders.
//!
//! Layout:
//! - [`sidecar`]: detect/install/spawn/health/teardown of llama-server.
//! - [`client`]: OpenAI-compatible SSE client, hard loopback-only guard.
//! - [`explain`]: EN/VI prompt construction + tolerant response parsing
//!   (pure, no network).

pub mod client;
pub mod explain;
pub mod sidecar;
