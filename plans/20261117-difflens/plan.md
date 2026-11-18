---
title: "DiffLens v1.0.0 — local-first document diffing desktop app"
description: "Tauri 2 + React desktop app comparing PDF/DOCX/TXT/XLSX/PPTX 100% locally with embedding paragraph alignment, heuristic intelligence with audit trail, optional local LLM explanations, experimental OCR, EN/VI i18n, and 3-OS CI releases."
status: pending
priority: P1
effort: 272h
issue: ""
branch: vibecoder880/feat/difflens-v1
tags: [feature, frontend, backend, infra, critical]
blockedBy: []
blocks: []
created: 2026-11-17
---

# DiffLens v1.0.0 Implementation Plan

## Context Links

- Stack research (authoritative): [research/researcher-01-stack-report.md](./research/researcher-01-stack-report.md)
- Accepted brainstorm contract (2026-11-17): outcome, constraints, non-goals, acceptance criteria (a)–(g) — settled, do not re-open.

## Overview

DiffLens v1.0.0 ships a production, MIT-licensed, cross-platform (Windows/macOS/Linux) Tauri 2 desktop app that compares PDF, DOCX, TXT, XLSX and PPTX documents 100% locally: extraction to a unified block model, `similar` word/char diff, 3-pass paragraph alignment (anchors → multilingual-e5-small ONNX embeddings via ort → move-repair window), deterministic VN+EN heuristic intelligence (money/date/percent/duration deltas + severity lexicon + full audit trail), optional local LLM explanations via a downloaded llama.cpp `llama-server` sidecar (Qwen3-4B-Instruct Q4_K_M, never bundled), experimental Tesseract-CLI OCR for scanned PDFs, EN+VI i18n, session history plus HTML/Markdown reports, and GitHub Actions CI building 3-OS release artifacts on tag push. No runtime network egress except the explicit user-initiated download manager.

## Key Insights (trade-offs carried from brainstorm/research)

- DOCX/PPTX parsed raw (zip + roxmltree) for run/order fidelity incl. tracked changes; fails first on exotic OOXML → fixture corpus + graceful degradation to text-only warnings.
- ort 2.0.0-rc.15 pinned `=2.0.0-rc.15`; inference hidden behind an `Embedder` trait so candle/tract fallback stays cheap.
- OCR v1 = detected Tesseract CLI only (VN quality honestly labeled experimental); bundled ONNX OCR is a NO-GO spike unless a quality gate passes.
- llama-server is a downloaded sidecar, not bundled; pinned llama.cpp b-tag + SHA256 manifest; fails first if a release drops binaries.
- VM disk constraint (9.5 GB free): local builds are Linux debug only; `CARGO_TARGET_DIR` relocated to ext4 cache + cargo-clean cadence; all release/3-OS builds run in CI (tauri-action matrix + free-disk-space).

## Architecture

Cargo workspace at repo root: `src-tauri/` thin Tauri shell (commands, state, events) + `core/` (`difflens-core`) headless crate with modules `model`, `extract` (txt/pdf/docx/xlsx/pptx), `diff`, `align` (anchors/embeddings/needleman), `heuristic` (entities/severity/audit), `llm` (sidecar/client/prompts), `ocr`, `download` (sole network egress point), `store`, `report` (both created in Phase 8). Frontend `src/` React 19 + Vite 7 + Tailwind 4 + shadcn/ui (Radix) + TanStack Router/Query/Virtual + i18next 27 (EN/VI). Bundled resources: PDFium binaries (bblanchon), e5-small ONNX + tokenizer (pinned commit). Optional downloads into `app_data_dir` (user-initiated, never bundled): llama-server, Qwen3 GGUF; bge-m3 int8 is strictly opt-in and ships only after the Phase 3 alignment gate is re-run with bge-m3 vectors. Persistence: rusqlite in `app_data_dir`.

## Phases

| # | Phase | File |
|---|-------|------|
| 1 | Repo bootstrap, toolchain, Tauri skeleton, CI/CD | [phase-01-bootstrap-ci.md](./phase-01-bootstrap-ci.md) |
| 2 | Core extraction layer (TXT/PDF/DOCX/XLSX/PPTX → block model) | [phase-02-extraction.md](./phase-02-extraction.md) |
| 3 | Diff + 3-pass alignment engine (similar, ort/e5-small, NW) | [phase-03-diff-alignment.md](./phase-03-diff-alignment.md) |
| 4 | Heuristic intelligence (VN/EN entities, severity, audit trail) | [phase-04-heuristics.md](./phase-04-heuristics.md) |
| 5 | llama-server sidecar + LLM explanations + download manager | [phase-05-llm-sidecar.md](./phase-05-llm-sidecar.md) |
| 6 | OCR path (Tesseract detection, rasterize→OCR, experimental flag) | [phase-06-ocr.md](./phase-06-ocr.md) |
| 7 | Frontend app (wizard, side-by-side view, change list, i18n, settings) | [phase-07-frontend.md](./phase-07-frontend.md) |
| 8 | Persistence + reports (rusqlite history, HTML/Markdown export) | [phase-08-persistence-reports.md](./phase-08-persistence-reports.md) |
| 9 | Release engineering (icons, 3-OS pipeline, E2E smoke, README, v1.0.0) | [phase-09-release.md](./phase-09-release.md) |

## Requirements (acceptance criteria mapping)

- (a) `cargo test` green incl. fixture extraction/diff/alignment tests → Phases 2–4, 6, 8, 9.
- (b) `npm run build` (pnpm runs the same script) + `cargo build` green locally on Linux → Phase 1, maintained thereafter.
- (c) GitHub Actions builds 3-OS artifacts + publishes a GitHub Release (draft ok) on tag push → Phases 1, 9.
- (d) End-to-end diff of two DOCX/PDF fixtures: side-by-side view, severity change list, audit trail, optional LLM explanation → Phases 2–5, 7.
- (e) EN+VI i18n switchable → Phase 7.
- (f) README with build instructions, screenshots, license + third-party attribution → Phase 9.
- (g) No runtime network egress except the download manager (grep gate on reqwest/fetch) → Phase 5 enforces, Phase 9 verifies.
- (h) Session history + HTML/Markdown report export with audit trail → Phase 8.

## Success Criteria

- [ ] All 9 phases complete with their success criteria green.
- [ ] Acceptance (a)–(g) verified; tag `v1.0.0` publishes a 3-OS draft release.
- [ ] Egress audit passes: network use confined to `core/src/download` plus loopback-guarded `core/src/llm/client.rs`.

## Risk Assessment

ort RC breakage → Embedder trait swap (candle/tract); unofficial community ONNX exports → pinned commits + SHA256 checks; VM disk pressure → Phase 1 hygiene; Vietnamese Tesseract quality → honest experimental labeling; llama.cpp release drift → pinned tag manifest; exotic OOXML → graceful degradation. Details per phase file.

## Security Considerations

All downloads user-initiated and SHA256-verified against a committed manifest; LLM HTTP client restricted to 127.0.0.1; zip-bomb/entry limits in extraction; updater private key only in GitHub secrets (never committed); no telemetry anywhere.

## Next Steps

1. Execute Phase 1 (toolchain + skeleton + CI), then proceed phase-by-phase in order.
2. After Phase 5 run the network-egress audit; after Phase 9 tag `v1.0.0` and verify the release.
