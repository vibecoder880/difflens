# Changelog

All notable changes to DiffLens are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [1.0.3] — 2026-11-22

Correctness fixes from the adversarial bug sweep (58 new edge-case tests).

### Fixed

- **Negative money and percentages now produce correct deltas** —
  `-1.000.000₫ → 1.000.000₫` and `-5% → -3%` previously computed deltas
  as if both values were positive (sign was dropped at extraction).
  Sign-glued hyphenation (`abc-5%`) is still treated as text, not a
  negative value.
- **Very large paragraphs no longer stall the diff** — the word-level
  similarity check was quadratic in input size; a pasted 500 KB paragraph
  could hang comparison for minutes. Inputs beyond 8 Ki-characters are
  now compared by bounded prefixes (500 KB pair: minutes → ~0.4 s).
- Extreme amounts (≥ 2^63) no longer saturate flag-detail formatting
  (the delta itself was already correct).

### Added

- 58 adversarial/edge-case tests: truncated and corrupt archives per
  format, random-bytes-with-PDF-magic, NUL/control characters, empty vs
  empty documents, 10 k-paragraph performance canaries, concurrent store
  writers, byte-flipped SQLite degradation, fragmented SSE frames,
  numeric boundary values, and a locale-canary guard.
- Locale check: pipeline output is byte-identical under `C` and
  `C.utf8` (no locale-dependent formatting leaks).

## [1.0.2] — 2026-11-19

Packaging fix release — found by hands-on testing with real documents.

### Fixed

- **PDFium now ships inside the app** (v1.0.0/v1.0.1 shipped without it, so
  PDF comparison failed on installed apps with "PDFium library not found").
  Each platform bundle carries its native PDFium library; the release
  pipeline verifies it landed inside the built bundle.
- **Real semantic alignment is now active in shipped apps** — the
  `embeddings-ort` feature was never enabled for the packaged shell, so
  releases silently used a deterministic fallback matcher and the bundled
  model was unused. The `multilingual-e5-small` ONNX model (~118 MB) now
  ships inside the bundle, and the release pipeline verifies it. The
  fallback remains as a safety net, never as the intended path.
- Vietnamese decimal parsing: `0,075%` was misread as `75.0%` (zero-head
  integer rule), making percent deltas wrong by three orders of magnitude
  on realistic contract values.
- macOS developer staging used the wrong library name in one fallback
  path (`.so` instead of `.dylib`).

### Added

- Hands-on QA suite as repeatable integration tests
  (`core/tests/qa_pipeline.rs`, `qa_loader_chain.rs`): the full pipeline
  over generated Vietnamese contract pairs in DOCX/PDF/XLSX/PPTX/Markdown,
  the bundled-resource resolution chain, and report/store roundtrips.
- Release builds now assert both the PDFium library and the e5-small model
  are present inside every built bundle before upload.

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
