---
title: "Phase 5: llama-server sidecar + LLM explanations + download manager"
status: todo
priority: P2
effort: 32h
---

# Phase 5: llama-server Sidecar + LLM Explanations + Download Manager

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.6 (llama.cpp releases, /v1/chat/completions, sidecar options, Qwen3-4B GGUF), §2.1 sidecar/spawn patterns; brainstorm: optional local LLM, downloaded on demand, never bundled.

## Overview

**Priority:** P2 · **Status:** pending · **Effort:** ~32h
Implement the optional local-LLM explanation feature end to end: a Rust download manager (the app's single network egress point) that fetches the pinned llama.cpp `llama-server` binary per OS and the Qwen3-4B-Instruct-2507 Q4_K_M GGUF with SHA256 verification and progress events; a sidecar manager that detects/downloads/spawns `llama-server` on 127.0.0.1 with health polling and clean teardown; a streaming chat client; EN/VI prompt templates that turn the severity-sorted change list into explanations streamed to the UI. Strict local-only guardrails: the client refuses any non-loopback endpoint.

## Key Insights

- llama.cpp `bNNNNN` releases ship per-OS archives containing `llama-server` (13–17 MB) — pin an exact b-tag + per-OS asset names + SHA256 in a committed manifest; brainstorm "fails first" clause: if a future release drops binaries, the manifest keeps a known-good tag working.
- Qwen3-4B-Instruct-2507 GGUF Q4_K_M ≈ 2.4–2.5 GB from `unsloth/Qwen3-4B-Instruct-2507-GGUF` (Apache-2.0) — user-initiated download with progress/cancel; UI shows the ≥8 GB RAM guidance.
- llama-server speaks OpenAI-compatible `POST /v1/chat/completions` with `stream: true` (SSE) — the only endpoint needed; embeddings stay in ort (Phase 3).
- Egress guardrail (acceptance g): `reqwest` exists only in `core/src/download` and `core/src/llm/client.rs`; the client hard-fails any non-loopback base URL and a unit test enforces it.

## Requirements

- [ ] `core/src/download`: job-based manager — user-initiated only, progress events, resume via Range, temp-file + atomic rename, SHA256 verify against committed manifest.
- [ ] Sidecar manager: PATH detection → else download per manifest → spawn `--host 127.0.0.1 --port <ephemeral> --api-key <random> -m <gguf>`; `/health` poll (60 s timeout); kill on Drop and app exit.
- [ ] Streaming client: SSE parse of chat completions, per-token Tauri events, cancel token, loopback-only guard (unit-tested).
- [ ] EN + VI prompt templates; prompt capped (top-N changes by severity, truncated blocks); strict "use only provided diff content" instruction; temperature 0.
- [ ] Hardened archive extraction (entry allowlist, no `..`/symlink/hardlink, size caps, staging + atomic rename) with zip-slip unit tests.
- [ ] `cargo test -p difflens-core -- download llm` green; egress grep audit passes.

## Architecture

`core/src/download/mod.rs` exposes `DownloadManager` (tokio, held in Tauri `State`) with `start_job(spec) -> job_id`, cancel, and progress events via `Emitter`; `core/src/download/checksum.rs` verifies SHA256. `core/src/download/manifest.json` (committed) pins, with per-entry provenance (source repo, tag/commit, license, checksum-source URL): llama.cpp b-tag + per-OS archive URLs/names/checksums, Qwen3-4B GGUF repo/file + checksum, a Qwen2.5-3B-Instruct Q4_K_M low-RAM fallback entry (runs in the 6 GB dev VM; exact filename/size/checksum verified at implementation — UNVERIFIED at planning), and an optional bge-m3 int8 entry (strictly opt-in download-only, never bundled; ships only after the Phase 3 gold alignment gate is re-run with bge-m3 vectors). Every fallback URL carries its own pinned checksum — fallbacks never share the primary checksum. `core/src/llm/sidecar.rs` owns process lifecycle in `app_data_dir/bin`; `core/src/llm/client.rs` is a minimal `reqwest` (rustls) SSE client whose `Client::new(base_url)` constructor rejects non-loopback; `core/src/llm/prompts.rs` renders prompts in EN or VI from `DiffResult` + `AuditTrail`.

## Related Code Files

- `core/src/download/mod.rs` (create — manager, jobs, progress, resume)
- `core/src/download/checksum.rs` (create — SHA256 verify)
- `core/src/download/manifest.json` (create — pinned tags/URLs/checksums)
- `core/src/llm/mod.rs`, `core/src/llm/sidecar.rs`, `core/src/llm/client.rs`, `core/src/llm/prompts.rs` (create)
- `src-tauri/src/commands.rs` (modify — add `llm_status`, `llm_install_server`, `llm_download_model`, `llm_explain`, `llm_cancel`; file created in Phase 3)
- `src-tauri/src/state.rs` (modify — add DownloadManager + LlmState to the Phase 3 Tauri State)
- `src-tauri/capabilities/default.json` (modify — event permissions for progress/token streams)
- `core/tests/download_tests.rs`, `core/tests/llm_tests.rs` (create)

## Implementation Steps

1. `download/checksum.rs`: streaming SHA256 (sha2 crate) with size cap; `download/mod.rs`: job struct {id, url, dest, expected_sha256, bytes_done/total}, tokio task writing to `dest.part` then atomic rename; Range-resume on restart; emit progress events at ≥1% deltas.
2. Write `manifest.json` with the pinned llama.cpp release (research-verified tag family b12484, pick current exact tag at implementation time) + per-OS asset patterns (`llama-<tag>-bin-ubuntu-x64.tar.gz`, `-win-cpu-x64.zip`, `-macos-arm64.tar.gz`), hardened archive extraction (extract by exact entry allowlist `**/llama-server` / `**/llama-server.exe`; reject absolute/`..`/symlink/hardlink entries; per-entry and total decompressed-size caps; extract into a staging dir then atomic-rename into `app_data_dir/bin`), the Qwen3-4B GGUF file spec, the Qwen2.5-3B-Instruct Q4_K_M low-RAM fallback entry, and the optional bge-m3 int8 entry (opt-in only, gated on the Phase 3 bge-m3 alignment re-run). Every entry records provenance (source repo, tag/commit, license, checksum-source URL) and every fallback URL carries its own pinned checksum (fallbacks never share the primary checksum); a manifest-integrity test asserts checksum + pinned version + provenance fields on every entry.
3. `sidecar.rs`: `detect()` checks PATH (`llama-server` on Unix, `llama-server.exe`) and common install locations; `install()` downloads via DownloadManager per OS manifest entry and extracts with the step-2 hardening; `spawn(model_path)` launches with `--host 127.0.0.1`, a random per-session `--api-key`, and ephemeral port resolution using startup-log parsing as the primary mechanism (regex on the logged bind line; unit test with a canned startup log; verify `--port 0` support at implementation time and record it in the manifest notes); `/health` poll with the api key until `{"status":"ok"}` or 60 s timeout, asserting 401 without the key (server-side bind verification post-spawn); process killed in `Drop` + on Tauri exit hook.
4. `client.rs`: `LlmClient::new(base_url)` — validate scheme + loopback host or return `Err(LocalOnlyViolation)`; `chat_stream(messages, model)` POST `/v1/chat/completions` `stream:true`, parse SSE `data:` frames into token deltas, forward via channel → Tauri event; cancel token aborts the request and kills nothing else.
5. `prompts.rs`: EN + VI system prompts (role: document-diff explainer; use only provided diff content; output JSON `{summary, explanations: [{change_id, explanation}], risk_note}`) + user template embedding top-N severity-sorted changes (id, op, severity, entity deltas, truncated texts, audit rationale keys); cap total prompt ≈6k tokens; temperature 0, `max_tokens` bounded.
6. Tauri commands + state: `llm_status` (server/model presence), `llm_install_server`, `llm_download_model` (both fan into DownloadManager with job ids), `llm_explain(session)` spawns sidecar if needed, streams tokens as `llm://token` events, emits `llm://done` with parsed JSON; `llm_cancel`.
7. Tests: loopback guard (http://example.com → Err; http://127.0.0.1, http://[::1] → ok); SSE parser against canned stream fixtures (incl. `[DONE]`, malformed frame); checksum mismatch → job fails, `.part` removed; resume test on a temp file; sidecar spawn `#[ignore]` test gated on an installed binary; manifest integrity test (checksum + version + provenance per entry); extraction hardening unit tests with a zip-slip fixture (`../` path entry, symlink entry, oversized entry → rejected; mirrors Phase 2's zip-bomb test pattern); startup-log port parsing against a canned llama-server startup log.
8. Egress audit script `scripts/audit-egress.sh`: `grep -rn "reqwest::\|Client::new\|fetch(" core/src src-tauri/src src --include="*.rs" --include="*.ts" --include="*.tsx" | grep -v "core/src/download\|core/src/llm/client.rs"` must be empty; wire into ci.yml as a step.

## Todo

- [ ] DownloadManager with progress/resume/SHA256 + manifest integrity test (incl. provenance fields)
- [ ] llama-server detect/install/spawn/health/teardown (hardened extraction + per-session api-key)
- [ ] Loopback-guarded SSE client + cancel
- [ ] EN/VI prompts with content cap + strict grounding instruction
- [ ] Tauri commands + events wired (status/install/download/explain/cancel)
- [ ] `scripts/audit-egress.sh` green in CI
- [ ] `cargo test -p difflens-core -- download llm` green

## Success Criteria

- `cargo test -p difflens-core -- download llm` exits 0 (no real network: tests use loopback mock server + fixtures).
- `scripts/audit-egress.sh` exits 0 on the repo.
- Manual gate (low-RAM path for the 6 GB dev VM): install server + the manifest's Qwen2.5-3B-Instruct Q4_K_M entry in the dev app, run explain on the DOCX fixture session — tokens stream in EN and VI, summary JSON parses, server process exits when the app closes. If a ≥16 GB machine is available, the gate may instead use the Qwen3-4B entry — either way, only manifest-pinned models are used.
- Zip-slip/extraction-hardening tests prove hostile archive entries are rejected; `/health` returns 401 without the per-session api key.
- Downloaded binaries/models verify against manifest SHA256s (mismatch test red-paths proven).

## Risk Assessment

- llama.cpp future release drops binary assets → manifest pins a known-good b-tag (brainstorm fails-first mitigation); update flow = bump manifest + checksums.
- GGUF mirror/repo availability → manifest supports a fallback URL per entry, each with its own pinned checksum; downloads are resumable.
- 6 GB VM RAM cannot run the 4B model locally for dev testing → dev tests use mock SSE server; the manifest includes a Qwen2.5-3B-Instruct Q4_K_M entry (pinned + checksummed) as the low-RAM verification model — real-model manual verification never uses ad-hoc models outside the SHA256 manifest.

## Security Considerations

- Only user-initiated downloads; every byte verified against committed SHA256 manifest before use; manifest entries carry provenance (repo, tag/commit, license, checksum-source URL) and fallback URLs carry their own pinned checksums.
- Archive extraction is hardened: exact entry allowlist (`**/llama-server`/`**/llama-server.exe`), rejection of absolute/`..`/symlink/hardlink entries, per-entry + total decompressed-size caps, staging dir + atomic rename; zip-slip fixture tests enforce it.
- llama-server runs loopback with a random per-session `--api-key`; `/health` asserts 401 without it; bind address verified post-spawn.
- LLM output is untrusted: rendered as plain text only (React escaping, no markdown-HTML rendering), length-capped, visually labeled "AI-generated, may be wrong" and kept visually distinct from deterministic audit entries (Phase 7); never persisted/exported into reports without that label (Phase 8 excludes it).
- LLM client restricted to 127.0.0.1 at construction (unit-tested); no document text ever sent off-machine.
- Model/server files live under `app_data_dir`; no `PATH` shadowing (absolute paths used to spawn).

## Next Steps

Phase 6 reuses the PDF rasterization path for OCR; Phase 7 wires this phase's commands/events into the downloads page and the LLM explanation panel.
