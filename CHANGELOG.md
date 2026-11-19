# Changelog

All notable changes to DiffLens are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [1.0.1] — 2026-11-19

Security hardening release. All 10 secondary findings from the v1.0.0 code
review are fixed. No new features, no behavior regressions.

### Security

- **API key off the command line** — the llama-server loopback session key is
  handed over via the `LLAMA_API_KEY` environment variable (verified against
  the pinned llama.cpp `b12503` binary) instead of a visible `--api-key`
  argument.
- **Loopback guard literal-only** — only literal `127.0.0.1` / `[::1]` base
  URLs are accepted; DNS names are rejected so a hostile resolver can never
  redirect the bearer token.
- **Bounded work** — HTTP 416 resume attempts capped at one; streaming
  explanations capped at 256 KiB; diff job map pruned to the last 50 jobs
  (running jobs never evicted).
- **Lock hygiene** — lock poisoning surfaces errors at command boundaries
  instead of panicking; the sidecar status lock is no longer held across the
  60-second spawn/health window.
- **Temp file hygiene** — OCR rasterizations are written owner-only (0600)
  and deleted on drop.
- **Key entropy** — session keys are 32 bytes from OS randomness (`getrandom`).

### Fixed

- HTTP 416 handling was unreachable (ureq surfaces 4xx as `Status` errors);
  resume-after-416 now actually works and persistent 416 fails cleanly.
- Download cancel now really aborts in-flight server/model downloads
  (`.part` kept for resume) — previously the UI cancel button was dead-wired.
- `run_diff` / `export_report` validate paths from the webview (existence,
  regular file, absolute export target with matching extension).
- Pipeline runs via `spawn_blocking` so a long diff cannot starve the async
  runtime.

### Changed

- GitHub Actions pinned by commit SHA (checkout, setup-node, rust-cache,
  tauri-action, toolchain); runner apt installs retry on mirror 503 outages.

## [1.0.0] — 2026-11-18

First production release.

### Added

- **Document extraction** — PDF (pdfium: text layer, per-char boxes,
  paragraph reconstruction), DOCX (raw OOXML: headings, tables, tracked
  changes `w:ins`/`w:del`), XLSX (calamine: cells + formula strings),
  PPTX (slide runs), TXT/Markdown; zip-bomb budgets and normalization
  (Unicode NFC, whitespace collapse).
- **Three-pass semantic alignment** — deterministic anchors (clause numbers,
  headings, unique money strings) → embedding cosine + Needleman–Wunsch
  monotonic alignment (`multilingual-e5-small` ONNX, mean pooling) →
  move-repair window; gold-corpus quality gate (precision ≥ 0.85,
  recall ≥ 0.80) enforced in CI with the real model.
- **Word-level diff** — inline spans (equal/insert/delete) via `similar`
  with block text carried on added/removed changes.
- **Deterministic heuristic intelligence** — Vietnamese + English entity
  extraction (money with triệu/tỷ scales, dates, durations, percent),
  delta computation (`MONEY_CHANGED`, `DURATION_CHANGED`, …), weighted
  severity lexicon with banded scores, and a full audit trail showing which
  rule fired with before/after evidence.
- **Local LLM explanations (optional)** — llama.cpp `llama-server` sidecar
  manager (pinned `b12503`, per-OS binaries with SHA-256 manifests),
  streaming `/v1/chat/completions` client with SSE parsing, English and
  Vietnamese prompts, loopback-only guard, download manager with resume +
  checksum-verified atomic install. Qwen3-4B-Instruct-2507 (primary) and
  Qwen2.5-3B-Instruct (fallback) GGUF models downloadable on demand.
- **OCR (experimental)** — scanned-PDF detection, Tesseract CLI path with
  `vie` language pack, TSV parsing + geometric line reconstruction, honest
  labeling and confidence warnings.
- **Desktop app** — Tauri 2, cross-platform (Windows/macOS/Linux);
  React 19 + Tailwind 4 UI with virtualized side-by-side diff view, change
  list with severity/kind filters, detail drawer with audit trail and
  streaming AI explanations, model download manager, settings (theme,
  locale, OCR preference); full English + Vietnamese i18n (210 keys with
  parity tests).
- **Persistence** — local SQLite session history (list/open/delete recent
  comparisons).
- **Reports** — standalone HTML and Markdown exports with hostile-content
  escaping golden tests.
- **CI/CD** — 6-job pipeline (core tests, embeddings with real model,
  shell compile, egress audit, real LLM smoke roundtrip, frontend) and
  tag-driven 3-OS release builds with checksums.

[1.0.1]: https://github.com/vibecoder880/difflens/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/vibecoder880/difflens/releases/tag/v1.0.0
