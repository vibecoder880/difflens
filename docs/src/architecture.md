# Architecture

A Tauri 2 desktop app: a plain Rust library (`difflens-core`) doing all the work, a thin shell (`src-tauri`) for IPC + persistence, and a React 19 frontend (`src`). The core is headless-testable with `cargo test`.

## Component diagram

```text
┌───────────────────────────── DiffLens (Tauri 2) ─────────────────────────────┐
│                                                                              │
│  React 19 + Tailwind + TanStack Router        src-tauri (thin shell)         │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────┐   ┌──────────────────────┐  │
│  │ Home/wizard  │  │ Diff report  │  │ Downloads│  │ commands.rs (IPC)    │  │
│  │ Settings     │  │ + detail     │  │ manager │──▶│ state.rs (sqlite     │  │
│  │ History      │  │ drawer       │  │ (only   │   │ session store)       │  │
│  └──────┬───────┘  └──────┬───────┘  │ egress  │   └──────────┬───────────┘  │
│         │                 │          │ in UI)  │              │              │
└─────────┼─────────────────┼──────────┴─────────┘──────────────┼──────────────┘
          ▼                 ▼                                   ▼
┌─────────────────────── difflens-core (Rust library) ─────────────────────────┐
│                                                                              │
│  extract/ ──▶ diff/ ──▶ align/ ──▶ heuristic/ ──▶ report/                    │
│  pdf (PDFium)  similar    e5-small      money/date/     HTML/Markdown         │
│  docx (zip+roxmltree)    semantic      percent/duration  export + audit      │
│  xlsx (calamine)         3-pass      rules (deterministic)  trail            │
│  pptx / txt                                                        │         │
│                                                                    ▼         │
│  embeddings/ (ort + ONNX e5)   llm/ (llama-server sidecar, loopback)         │
│  ocr/ (optional Tesseract CLI) store/ (rusqlite sessions)                    │
│                                                                              │
│  download/ — the ONLY module allowed to touch the network                    │
│  (manifest-pinned URLs, SHA-256 verified; enforced by scripts/audit-egress.sh│
│   and the egress-audit CI job)                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Workspace layout

```text
/
├── core/                  # difflens-core (MSRV 1.88)
│   ├── src/extract/       # pdf / docx / xlsx / pptx / txt (+ zip budgets)
│   ├── src/align/         # anchors.rs, needleman.rs, 3-pass orchestration
│   ├── src/diff/          # similar-based word diff, stats
│   ├── src/heuristic/     # entities, deltas, severity, labels (i18n keys)
│   ├── src/embeddings/    # Embedder trait, ort e5 (feature), fake embedder
│   ├── src/llm/           # sidecar lifecycle, SSE client, explain prompts
│   ├── src/download/      # DownloadManager, manifest, archive gates (only egress)
│   ├── src/ocr/           # detect, rasterize, engine (Tesseract path)
│   ├── src/store/         # rusqlite session history
│   ├── src/report/        # html / markdown exporters + audit trail
│   └── src/pipeline.rs    # extract → ocr → align → diff → enrich
├── src-tauri/             # commands.rs (IPC), state.rs (jobs + sqlite)
├── src/                   # React 19 + TS: routes/, components/, lib/, i18n/
├── scripts/               # fetch-pdfium.sh, fetch-models.sh, audit-egress.sh
└── resources/manifests/   # llm-manifest.json — the only allowed download URLs
```

## Tech stack

| Component | Chosen | Version | License |
|---|---|---|---|
| Desktop framework | Tauri 2 | 2.12.6 | MIT OR Apache-2.0 |
| PDF | pdfium-render / PDFium build | 0.9.5 / chromium 8165 | MIT OR Apache-2.0 / BSD-3 |
| DOCX/PPTX | zip + roxmltree | 9.7.0 / 0.21.2 | MIT / MIT OR Apache-2.0 |
| XLSX | calamine | 0.38.0 | MIT |
| Diff engine | similar | 3.2.1 | Apache-2.0 |
| Embeddings runtime | ort (ONNX Runtime) | 2.0.0-rc.15 | MIT OR Apache-2.0 |
| Embedding model | multilingual-e5-small ONNX | pinned commit | MIT |
| Tokenization | tokenizers | 0.23.2 | Apache-2.0 |
| LLM server | llama.cpp llama-server | b12503 | MIT |
| Models | Qwen3-4B / Qwen2.5-3B GGUF | current | Apache-2.0 |
| OCR | Tesseract CLI (not bundled) | 5.x | Apache-2.0 |
| Persistence | rusqlite + SQLite | 0.32.1 | MIT / public domain |
| HTTP (download only) | ureq | 2.12.1 | MIT OR Apache-2.0 |
| Frontend | React / TypeScript / Vite / Tailwind | 19.3.1 / 7.0.2 / 8.3.8 / 4.3.5 | MIT (TS: Apache-2.0) |
| UI libs | TanStack Query/Router/Virtual, i18next | 5.108.0 / 1.170.46 / 3.14.26 / 27.1.5 | MIT |

Full attributions: [THIRD-PARTY-NOTICES.md](https://github.com/vibecoder880/difflens/blob/main/THIRD-PARTY-NOTICES.md). No AGPL/GPL components.

## Data flow

1. `run_diff` validates paths → job registered (`job-*`) → pipeline runs on the **blocking pool**.
2. Progress callback → `diff://progress` events (`extract → ocr → align → diff → enrich`, terminal `done/failed/cancelled`); `cancel_diff` sets a flag honored at stage boundaries.
3. Two `DocumentModel`s extracted (OCR may append provenance-marked blocks) → 3-pass alignment → word diff → heuristic enrich (flags + severity + audit).
4. `DiffResult` persisted fire-and-forget to SQLite; served by `get_diff`; `session-*` ids hydrate history.
5. Optional `explain_change` → loopback sidecar → `llm://token` stream → `llm://done`. Never alters the result.
6. `export_report` writes HTML/Markdown to a validated path.

## IPC surface

| Command | Purpose |
|---|---|
| `run_diff(old, new, ocr?)` | start a diff job → snapshot |
| `get_diff(job_id)` | poll snapshot; `session-*` hydrates history |
| `cancel_diff(job_id)` | flag cancel at stage boundaries |
| `list_sessions` / `get_session` / `delete_session` | history CRUD |
| `export_report(id, format, target, lang?)` | standalone HTML/MD report |
| `ocr_status()` | OCR availability snapshot |
| `llm_status()` | sidecar + model snapshot |
| `llm_install_server()` | pinned sidecar install (`server-<tag>`) |
| `llm_download_model(model_id)` | SHA-256-gated GGUF download (`model-<id>`) |
| `explain_change(job, change, lang)` | streamed explanation (`explain-*`) |
| `llm_cancel(job_id)` | cancel explain or download |

| Event | Payload | Phase |
|---|---|---|
| `diff://progress` | `{job_id, stage, percent}` | pipeline |
| `llm://download` | `{job_id, kind, model_id, state, bytes_done, bytes_total?, percent, error?}` | installs |
| `llm://token` | `{job_id, delta}` | explanation stream |
| `llm://done` | `{job_id, ok, cancelled, summary?, impact?, error?, elapsed_ms}` | explanation terminal |
