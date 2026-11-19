---
title: "fix: v1.0.0 code-review P2 hardening (10 findings) on difflens main"
date: 2026-11-19
summary: All 10 P2 findings from plans/20261117-difflens/reports/code-review-v1.0.0.md fixed in 4 conventional commits; 6/6 CI jobs green (run 44506499253).
---

# fix: v1.0.0 code-review P2 hardening (10 findings) on difflens main

## What happened
Ran ak-fix --auto against the v1.0.0 review P2 list (HEAD 82e892d). Four commits pushed to main (2874647..d188a4b):

1. 2874647 fix(core): P2-2 loopback guard now accepts only literal 127.0.0.1/[::1] (localhost moved to the rejection list — resolver-poisoning threat while the bearer key is attached); P2-7 chat_stream caps accumulated text at 256 KiB (MAX_EXPLANATION_BYTES), error "explanation exceeded size limit", proven by a fake loopback SSE server streaming ~10 MB with no [DONE].
2. fd822c0 fix(core): P2-4 ensure_running no longer holds the manager mutex across spawn (short-lock decision/swap phases; active_info never stalls behind a 60 s spawn; behavioral note: a failed spawn no longer leaves a dead entry behind); P2-10 random_api_key = 32 CSPRNG bytes via getrandom 0.3 (64 hex chars, time/pid/ASLR counter scheme removed, sha2 fallback only on OS-entropy error); P2-1 verified pinned llama.cpp b12503 supports `--api-key KEY (env: LLAMA_API_KEY)` (+ `--api-key-file`), so the key now goes via env, never argv (config-file hand-off rejected: persists secret to disk).
3. a4a093b fix(core): P2-6 OCR temp TIFFs via tempfile::NamedTempFile (0600 on unix, Drop-delete; %TEMP%/difflens-ocr/ directory removed; tempfile moved dev-deps -> deps); P2-9 core: DownloadManager poison-safe (register/cancel/snapshot/start -> CoreResult, finish/thread bookkeeping degrade to tracing warns); P2-3 ureq 2.12.1 redirect semantics verified from vendored source (5 hops default, cross-host, AuthPolicy::Never strips auth on redirect) and documented in module docs + pinned .redirects(5) in build_llm_agent.
4. d188a4b fix(tauri): P2-5 job map pruned on insert to MAX_JOBS=50 (oldest finished first via insertion seq, never evict Running); P2-9 shell: DiffState/ExplainJobs poisoning-proof (command-boundary methods return Result; fire-and-forget methods warn); P2-8 run_diff uses async_runtime::spawn_blocking. Also removed a stale job-id assertion ("job-0") that contradicted next_job_id — src-tauri tests are compile-checked only (CI has no cargo test -p difflens).

## Decision
- P2-1: env hand-off chosen over api-key-file (no disk persistence). Runtime proof = CI llm-smoke real roundtrip passing with the env-var key.
- P2-3: docs-only + explicit .redirects(5) (trivial builder option); restricting hops further judged unnecessary given pinned-URL + SHA-256-before-rename + credential-free downloads.
- No separate docs commit: threat-model notes live inline in module docs where they are maintained.
- Verified-good list untouched: checksum ordering, archive gates, zip budgets, SSE parser resilience, HTML escaping all preserved (only cap added at the accumulation site).

## Verification
- Local: cargo fmt --check clean, clippy -p difflens-core --all-targets -D warnings clean, core tests 237 unit + 48 integration passed, vitest 52/52, egress audit ok. src-tauri not compilable locally (no pkg-config/webkit2gtk) — rode on CI.
- CI run 44506499253: 6/6 green (core, embeddings, shell compile, egress-audit, llm-smoke real roundtrip, frontend).

## Next steps
- P1 findings from the same review remain open (dead download cancel, action pinning etc. — P1-5 capability trim already landed in b7bea08).
- If credentials are ever added to downloads or user-supplied URLs allowed, revisit the ureq redirect policy per the new module-doc note.

> Historical work record — not durable authority. Prefer docs/specs/ADRs for current decisions.
