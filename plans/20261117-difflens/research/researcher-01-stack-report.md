# DiffLens Stack Research Report (researcher-01)

- **Date:** 2026-11-17
- **Scope:** Technology selection for a Tauri 2 + Rust + React document-comparison app (PDF/DOCX/TXT/XLSX/PPTX, local embeddings, optional local LLM, OCR, i18n EN+VI, MIT license).
- **Build constraints considered:** Linux VM, 3 cores, 6 GB RAM, ~9.5 GB free disk; GitHub Actions CI for Windows/macOS/Linux.
- **Method:** WebSearch + WebFetch against crates.io API, GitHub releases/API, HuggingFace API, official docs. Everything below is verified unless marked **UNVERIFIED**.

---

## 1. Decision table

| # | Component | Chosen lib / model | Version (Nov 2026) | License | App-license OK? | Confidence |
|---|-----------|--------------------|--------------------|---------|-----------------|------------|
| 1 | Desktop framework | **Tauri 2** (`tauri`, `tauri-build`, CLI) | **2.12.6** (published 2026-11-16; 3.0 still alpha) | MIT OR Apache-2.0 | Yes | High |
| 2 | CI release builds | **tauri-action** (GitHub Marketplace) with 3-OS matrix | 2.x line (exact tag UNVERIFIED) | MIT | Yes | High |
| 3 | PDF text extraction | **pdfium-render** + bundled **bblanchon/pdfium-binaries** dynamic libs | pdfium-render **0.9.5** (2026-11-01) | MIT OR Apache-2.0 (crate); PDFium itself **BSD-3-Clause**; binaries repo MIT | Yes (BSD-3 is MIT-compatible; must ship attribution) | High |
| 4 | DOCX parsing | **`zip` + `roxmltree`** raw parse of `word/document.xml` (own parser for paragraphs/runs/tables/`w:ins`/`w:del`) | zip **9.7.0** (MIT); roxmltree **0.21.2** (MIT OR Apache-2.0) | MIT / MIT-OR-Apache | Yes | High |
| 5 | XLSX reading | **calamine** | **0.38.0** (2026-11-02) | MIT | Yes | High |
| 6 | PPTX reading | **`zip` + `roxmltree`** on `ppt/slides/slideN.xml` (`<a:t>` runs) — no dominant maintained crate exists | (same zip/roxmltree) | MIT / MIT-OR-Apache | Yes | High |
| 7 | Embeddings runtime | **ort** (ONNX Runtime bindings) | **2.0.0-rc.15** (2026-10-19; wraps ONNX Runtime ~v1.31.0 per project badge) | MIT OR Apache-2.0 | Yes | High (rc caveat) |
| 8 | Default embedding model | **Xenova/multilingual-e5-small** ONNX export (base: intfloat/multilingual-e5-small, 118M params, 384-dim) | current | **MIT** (base model card) | Yes | High |
| 9 | Optional larger model | **bge-m3 ONNX** community export (Xenova/bge-m3, aapot/bge-m3-onnx, gpahal/bge-m3-onnx-int8; 568M params, dense 1024-dim) | current | **MIT** (BAAI/bge-m3) | Yes | Medium-High (community exports, not official) |
| 10 | Tokenization | **tokenizers** crate (HF) loading `tokenizer.json` + mean pooling in our code | **0.23.2** stable (1.0.0-rc.4 exists, do not use) | Apache-2.0 | Yes | High |
| 11 | Local LLM server | **llama.cpp `llama-server`** sidecar, OpenAI-compatible `/v1/chat/completions` **and `/v1/embeddings`** | latest nightly-style tag **b12484** (2026-11-17); pin a specific b-tag | **MIT** | Yes | High |
| 12 | Explanation GGUF | **Qwen3-4B-Instruct-2507 GGUF Q4_K_M** (~2.4–2.5 GB) via unsloth/Qwen3-4B-Instruct-2507-GGUF or Qwen official | current | **Apache-2.0** | Yes | High |
| 13 | Diff engine | **similar** (Myers/Patience, inline diff support) | **3.2.1** (2026-10-24) | Apache-2.0 | Yes (attribution required) | High |
| 14 | Word/char diff helper | **diff-match-patch-rs** (optional, for semantic word-level cleanup) + **strsim 0.11.1** (MIT) / **levenshtein 1.0.5** (MIT) for similarity scores | 0.5.1 | MIT OR Apache-2.0 | Yes | High |
| 15 | OCR | **v1: NO bundled OCR engine.** Ship text-layer extraction first; optional user-installed **Tesseract CLI** detection (`tesseract-ocr-vie` lang pack); PaddleOCR-class via ort deferred to v2 | tesseract 5.x (Apache-2.0) | Apache-2.0 | Yes | Recommendation: High |
| 16 | Frontend | **React 19 + TypeScript + Vite 7 + Tailwind CSS 4.x + shadcn/ui (Radix) + TanStack Query 5.x + TanStack Router + i18next 27** | Tailwind 4.3.5; TanStack Query 5.107.x; i18next 27.1.4; Router 1.x line | MIT (all) | Yes | High |
| 17 | App license | MIT | — | — | Compatible stack | High |

---

## 2. Per-area findings

### 2.1 Tauri 2.x

- **Stable:** `tauri` 2.12.6 on crates.io (published 2026-11-16). Tauri 2.12 announced 2026-09-26 as the "biggest update in the 2.x series". Tauri 3.0 exists only as `3.0.0-alpha.6` — do **not** target it.
- **Linux system deps:** Tauri 2 uses **webkit2gtk-4.1** (not 4.0). Ubuntu 24.04/Debian 13 dropped `libwebkit2gtk-4.0-dev`, so v1 is broken there; v2 is fine. Install: `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev` (plus `libgtk-3-dev` pulled transitively). This is what CI (ubuntu-24.04 runner) and the dev VM need.
- **tauri-action:** official action builds all 3 desktop OSes from a matrix, creates/updates a GitHub Release, and auto-generates `latest.json` + `.sig` files when updater secrets are present. Without secrets it still builds and uploads unsigned artifacts.
- **Updater signing:** the updater plugin verifies signatures — you cannot use the updater without them. But **signing is optional overall**: if you omit the updater plugin/config, no keys are needed; an OSS project can generate keys with `tauri signer generate` and store `TAURI_SIGNING_PRIVATE_KEY` as a repo secret (free, no CA involved). Separate issue: OS-level code signing (Apple Developer ID, Windows Authenticode) is not required to distribute, but unsigned builds trigger Gatekeeper/SmartScreen warnings — acceptable for OSS v1, document "right-click → open" on macOS.
- **Sidecar API:** `bundle.externalBin: ["binaries/llama-server"]` in `tauri.conf.json`; binaries must carry the target-triple suffix (e.g. `llama-server-x86_64-pc-windows-msvc.exe`); spawn via `tauri-plugin-shell` — `app.shell().sidecar(...)` from Rust or `Command.sidecar()` from JS. Capabilities need shell permissions. For models/GGUF downloaded on demand, no sidecar needed — write to `app_data_dir` and reference by path.
- **Disk usage:** a Tauri/Rust `target/` routinely grows to **10–20 GB** in CI; GH Actions runners (~14 GB free) can OOM the disk — use `jlumbroso/free-disk-space` or selective cleanup, cache `~/.cargo` but not `target/`. For the local 9.5 GB-free VM: a single-platform debug build fits (~5–8 GB) but is tight; do release/3-OS builds only in CI, and consider `CARGO_TARGET_DIR` relocation or `cargo clean` between debug/release.

**Sources:** https://v2.tauri.app/blog/tauri-2-12/, https://v2.tauri.app/release, https://v2.tauri.app/start/prerequisites/, https://v2.tauri.app/distribute/pipelines/github, https://v2.tauri.app/plugin/updater, https://v2.tauri.app/develop/sidecar, https://github.com/tauri-apps/tauri-action, https://crates.io/api/v1/crates/tauri, https://stackoverflow.com/questions/79309374/

### 2.2 PDF in Rust

- **pdfium-render 0.9.5** (2026-11-01, MIT OR Apache-2.0, https://github.com/ajrcarey/pdfium-render). It does **not** compile PDFium; it loads `libpdfium.so`/`pdfium.dll`/`libpdfium.dylib` at runtime via `libloading` (`Pdfium::bind_to_library(...)` or bind-to-system).
- **Text extraction with positions:** confirmed APIs for per-character boxes — `PdfPageText::chars()` yields `PdfPageTextChar` with `char_box()` (precise `PdfRect`) and `loose_box()`; `PdfPageTextSegment` (with `chars()`) exists for segment-level grouping. Paragraph/line reconstruction must be built on top of char boxes (standard Y-cluster/X-order algorithm) — plan ~1 week of work for robust line/paragraph grouping with rotated-text edge cases.
- **Bundling strategy:** use **bblanchon/pdfium-binaries** releases (weekly automated builds since 2017; MIT repo). Assets are per-OS `.tgz`: `pdfium-win-x64.tgz`, `pdfium-mac-arm64.tgz` / `pdfium-mac-univ.tgz`, `pdfium-linux-x64.tgz` (+musl variants). Ship the `.so/.dll/.dylib` inside the Tauri bundle (`bundle.resources` or per-platform `externalBin`-style resource naming) and call `bind_to_library` with the resolved resource path. Each archive is ~5–10 MB uncompressed; ~10–25 MB added per installer.
- **PDFium license:** BSD-3-Clause-style ("Copyright 2014 The PDFium Authors") — MIT-compatible; include the notice in your about/licenses screen.

**Sources:** https://crates.io/crates/pdfium-render, https://github.com/ajrcarey/pdfium-render, https://github.com/bblanchon/pdfium-binaries, https://raw.githubusercontent.com/chromium/pdfium/main/LICENSE

### 2.3 DOCX in Rust

- `docx-rs` (bokuweb, MIT) is a read **and** write library, and its README claims parsing support for tracked changes, footnotes, comments, hyperlinks via `read_docx`. However, its `Docx` model is writer-oriented; relying on it for diff-grade fidelity (exact run splitting, `w:ins`/`w:del` *ordering within a paragraph*) is risky — the model is not designed for position-faithful reconstruction.
- `docx` crate (0.1.x) and various extraction crates exist but none dominate; general-purpose crates historically handle tracked changes poorly.
- **Recommendation: parse it yourself** — a DOCX is a zip: open with `zip` crate, read `word/document.xml`, parse with `roxmltree` (tree API is the right fit here; document.xml is typically < a few MB). Map: `w:p` → paragraph, `w:r` → run, `w:t` → text, **`w:delText`** inside `w:del` (deleted text lives in `w:delText`, not `w:t`), `w:tab`, `w:br`, hyperlinks (`w:hyperlink`), tables (`w:tbl`/`w:tr`/`w:tc`). For tracked-changes-aware diff: render `w:ins` as accepted-inserted text, `w:del` as accepted-removed (or expose a toggle). ~500–800 lines of Rust, deterministic, zero license concerns, full control. This is the approach the search evidence supports as best for review-oriented extraction.

**Sources:** https://raw.githubusercontent.com/bokuweb/docx-rs/master/README.md, https://docs.rs/roxmltree, https://crates.io/keywords/docx, https://betterprogramming.pub/how-to-parse-microsoft-word-documents-docx-in-rust-d62a4f56ba94

### 2.4 XLSX and PPTX in Rust

- **calamine 0.38.0** (2026-11-02, MIT, pure Rust) reads `.xlsx/.xlsm/.xlsb/.xls/.ods`. Reads cell **values** (Excel-cached results) and exposes formula strings, but does **not** evaluate formulas. For diff: compare cached values + formula text; flag "formula changed, cached value same" cases. MSRV 1.88.
- **PPTX:** no dominant maintained `pptx` crate; the established pattern (used by undoc, transmutation, Kreuzberg) is `zip` + XML parsing of `ppt/slides/slideN.xml`, extracting `<a:t>` runs in order, plus `ppt/presentation.xml` slide ordering. Fully feasible in ~200 lines. Handle notes slides (`ppt/notesSlides/`) as optional extra.

**Sources:** https://crates.io/api/v1/crates/calamine, https://github.com/tafia/calamine, https://lib.rs/crates/undoc, https://crates.io/crates/transmutation

### 2.5 Embeddings in Rust

- **ort 2.0.0-rc.15** (2026-10-19, MIT OR Apache-2.0, pyke.io). ~26.6M total downloads. Wraps ONNX Runtime; project badge references **onnxruntime v1.31.0** (current); 2.0 supports multiversioning so it can run against older ORT 1.x versions (earlier reporting said 1.17→1.24 range; exact supported floor UNVERIFIED). It is still a release candidate after ~2 years of RCs — pin `=2.0.0-rc.15`, test on all 3 OSes, and be prepared to stay pinned. Default `download-binaries` feature fetches prebuilt ORT per target at build time (adds ~100–300 MB build download; runtime `.so/.dll/.dylib` ~10–25 MB — must be bundled or pulled via the feature's static linking option).
- **Models on Hugging Face:**
  - `Xenova/multilingual-e5-small` — ONNX weights confirmed (quantized variant flagged in repo metadata), `tokenizer.json` included (Xenova exports are Transformers.js-compatible; tokenizer files present — filenames beyond standard layout UNVERIFIED). Base model `intfloat/multilingual-e5-small`: **MIT** (verified via HF API), 12 layers, 118M params, 384-dim embeddings.
  - `bge-m3` (BAAI, **MIT** verified via HF API; 568M params, dense 1024-dim + sparse + ColBERT heads): multiple community ONNX exports — `Xenova/bge-m3`, `aapot/bge-m3-onnx`, `gpahal/bge-m3-onnx-int8`, `philipchung/bge-m3-onnx`. **Caveat:** standard Optimum exports capture only the dense head; a custom export script is needed for sparse/ColBERT (aapot provides one). For DiffLens, dense-only is sufficient → use Xenova or aapot export. None of these exports are maintained by BAAI — pin to a specific commit hash.
- **Tokenization + pooling:** use `tokenizers` crate (0.23.2 stable; Apache-2.0) loading the exported `tokenizer.json`. E5 protocol: prepend `"query: "` / `"passage: "` (Vietnamese prefixes work as-is; e5 is multilingual). Mean pooling over the last hidden state with the attention mask, then L2-normalize — trivial to implement in Rust (~30 lines).
- **CPU speed (3 cores):** e5-small-class models (12 layers, 384 hidden) with int8/fp16 ONNX on 3 modern x86 cores: roughly **5–20 ms per sentence at ≤128 tokens, 50–150 ms at 512 tokens** (order-of-magnitude estimate from ONNX Runtime CPU benchmarks for similarly sized models; exact 3-core numbers UNVERIFIED). A 1,000-paragraph document ≈ 5–30 s worst case with 512-token inputs; batch by sorting length + `intra_op_num_threads=3`. **bge-m3 (568M)** on 3 CPU cores: expect **~30–100 ms per short sentence, several hundred ms at 512 tokens** — usable for ≤500-paragraph docs, sluggish beyond; ship it as opt-in with a warning, and prefer the int8 export (gpahal) to halve the ~2.2 GB fp32 model (int8 ≈ 600 MB; sizes are arithmetic estimates from param counts, UNVERIFIED exact).

**Sources:** https://crates.io/crates/ort, https://ort.pyke.io, https://github.com/pykeio/ort, https://huggingface.co/Xenova/multilingual-e5-small, https://huggingface.co/intfloat/multilingual-e5-small, https://huggingface.co/api/models/intfloat/multilingual-e5-small, https://huggingface.co/api/models/BAAI/bge-m3, https://huggingface.co/aapot/bge-m3-onnx, https://huggingface.co/gpahal/bge-m3-onnx-int8, https://huggingface.co/Xenova/bge-m3, https://manticoresearch.com/blog/onnx-embeddings-speedup, https://arxiv.org/abs/2402.05672

### 2.6 llama.cpp `llama-server`

- **Releases:** `ggml-org/llama.cpp` publishes automated `bNNNNN` releases continuously; latest checked **b12484** (2026-11-17). Every build ships **full binary archives per OS** — `llama-b12484-bin-ubuntu-x64.tar.gz`, `llama-b12484-bin-win-cpu-x64.zip` (+arm64), `llama-b12484-bin-macos-arm64.tar.gz` (17 MB) / `-macos-x64.tar.gz` (13 MB) — all include **`llama-server`**. Pin an exact b-tag (nightly tags move fast; verify GPU-off CPU build works on your targets). License: **MIT** (ggml-org/llama.cpp).
- **API:** official README lists OpenAI-compatible endpoints including **`POST /v1/chat/completions`** and **`POST /v1/embeddings`** (plus `/v1/models`, `/v1/rerank`, health). Note `/v1/embeddings` is the OAI-compatible route; `/embedding` (singular) is llama.cpp-specific with more pooling options. Server startup flags: `--embedding`/`--embeddings` restricts to embedding mode; `--embd-normalize` controls normalization. For DiffLens, llama-server is used for chat explanations only (embeddings go through `ort`), so only `/v1/chat/completions` matters, plus `--host 127.0.0.1 --port <random>` and health polling (`/health`).
- **Sidecar management in Tauri:** two options — (a) bundle `llama-server` as `externalBin` sidecar (~10–40 MB per OS inside installer, always available), or (b) download on demand into `app_data_dir/bin` with checksum verification and spawn as an ordinary child process (recommended: keeps installer small; the task spec says GGUF downloaded on demand — the server binary can also be bundled cheaply given 13–17 MB archives). Manage lifecycle from Rust: spawn, poll `/health`, kill on exit. Community plugin `tauri-plugin-sidecar` adds health checks/auto-restart if wanted.
- **GGUF model choice (explanation task, EN+VI):**
  - **Qwen3-4B-Instruct-2507** (Apache-2.0): Q4_K_M ≈ **2.4–2.5 GB**; strong multilingual incl. Vietnamese; best default. Use `unsloth/Qwen3-4B-Instruct-2507-GGUF` (well-maintained GGUFs) or `Qwen/Qwen3-4B-GGUF`.
  - **Qwen2.5-3B-Instruct** (Apache-2.0): Q4_K_M ≈ 1.8–2.0 GB; lighter fallback for 8 GB-RAM machines.
  - **Llama-3.2-3B-Instruct**: Q4_K_M ≈ 1.9 GB but **Llama 3.2 Community License** (custom acceptable-use + branding terms) — not a license trap for redistribution per se, but for an MIT-licensed OSS app prefer Apache-2.0 Qwen models; offer Llama only as user-supplied local file if at all.
  - With 6 GB RAM build machine, note: end users typically need ≥8 GB RAM for the 4B Q4 model comfortably; ship model-size guidance in the UI.

**Sources:** https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=3, https://github.com/ggml-org/llama.cpp/releases, https://raw.githubusercontent.com/ggml-org/llama.cpp/master/tools/server/README.md, https://huggingface.co/Qwen/Qwen3-4B-GGUF, https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF, https://huggingface.co/Qwen/Qwen3-4B-Instruct-GGUF (license: apache-2.0 family), https://llm-stats.com (Qwen2.5 Apache-2.0 vs Llama Community License), https://v2.tauri.app/develop/sidecar, https://github.com/tauri-apps/plugin-shell

### 2.7 OCR in Rust — honest assessment

- **What "PaddleOCR-class via ort" means:** three models (DBNet text detection ~4.7 MB mobile, angle classifier ~1.4 MB, CRNN/SVTR recognizer ~10–15 MB mobile) + preprocessing (resize/normalize to fixed aspect), DBNet bitmap post-processing (binarize → find contours → unclip polygons — needs connected-components; no mature pure-Rust DBNet postproc crate; you'd port ~300 lines of C++/Python), CTC decoding, then line grouping into reading order. Community ONNX models exist: `monkt/paddleocr-onnx` (16 models, 48+ languages incl. PP-OCRv5), `xberg-io/paddleocr-onnx-models`, RapidOCR (RapidAI) publishes official PP-OCRv4/v5/v6 ONNX det/cls/rec with a language matrix. **Vietnamese recognition specifically:** PP-OCRv5 `latin_PP-OCRv5_mobile_rec` covers Vietnamese (Latin script, vi dict); community vi-specific rec models exist (e.g. `tieubaoca/pp-ocrv6-medium-rec-vietnamese` on HF, paddle format → needs `paddle2onnx`); RapidOCR docs list vi support. Quality of any of these on real Vietnamese scanned docs is **UNVERIFIED — no independent benchmarks found**.
- **Complexity verdict:** running det+rec via `ort` is technically feasible (C#/C++/Python projects do it; Rust has no turnkey crate) but it is a **multi-week engineering effort** with real risk on the polygon-postprocessing and reading-order side, plus per-model licensing needs checking (PaddleOCR models: Apache-2.0 for code; model weights generally Apache-2.0 but verify per model — UNVERIFIED per-file).
- **Fallback option:** detect `tesseract` on PATH (Apache-2.0 since v4; `tesseract-ocr-vie` language package exists in Ubuntu/Debian repos; `vi` traineddata in tessdata/tessdata_fast on GitHub). Spawn CLI, hOCR/plain-text out. Zero bundling burden; quality on Vietnamese is mediocre-but-known (diacritics errors on low-res scans).
- **GO/NO-GO for v1: NO-GO for bundled OCR.** Recommendation: v1 ships (1) text-layer extraction via pdfium for digital PDFs, (2) a "scanned PDF detected" message, (3) optional **Tesseract CLI integration** (use if installed; point Windows/macOS users at installers), (4) OCR via ONNX as a v2 feature gated behind a design spike in Q1. Rationale: OCR is orthogonal to the core diff value proposition; embeddings already give the hard part (scanned-vs-scanned alignment is hopeless anyway without OCR on both sides — scope v1 to scanned-vs-digital or scanned-view-only).

**Sources:** https://github.com/rapidai/rapidocr, https://rapidai.github.io/RapidOCRDocs/, https://huggingface.co/monkt/paddleocr-onnx, https://huggingface.co/xberg-io/paddleocr-onnx-models, https://huggingface.co/tieubaoca/pp-ocrv6-medium-rec-vietnamese, https://paddlepaddle.github.io/PaddleOCR/main/en/version3.x/algorithm/PP-OCRv5/PP-OCRv5_multi_languages.html, https://github.com/BobLd/RapidOcrNet, https://arxiv.org/html/2510.04003v1

### 2.8 Diff / alignment crates

- **similar 3.2.1** (2026-10-24, Apache-2.0, mitsuhiko) — dependency-free, Myers + Patience (git-like), `TextDiff` with line/word/char granularity and **inline diffs** on replace ops (`iter_inline_changes`); used by difftastic, delta, cargo-public-api. Note the major bump: 2.x → 3.x happened along the way; use 3.x docs.
- **diff-match-patch-rs 0.5.1** (MIT OR Apache-2.0) — fast Myers-based DMP port with semantic cleanup; good for char-level word diffs if `similar`'s granularity isn't what you want. `dmp` (SurrealDB, 2.3M downloads) is an alternative.
- **strsim 0.11.1** (MIT) / **levenshtein 1.0.5** (MIT) for token-similarity scores feeding the severity heuristic.
- **Recommendation:** paragraph alignment = embedding cosine similarity (Hungarian/greedy assignment, our code) + `similar` for word-level diff within a changed paragraph-pair and char-level inline changes; keep `diff-match-patch-rs` only if cleanup semantics are needed. `similar` is the one crate to standardize on.

**Sources:** https://crates.io/api/v1/crates/similar, https://github.com/mitsuhiko/similar, https://docs.rs/similar, https://crates.io/api/v1/crates/diff-match-patch-rs, https://crates.io/api/v1/crates/strsim, https://crates.io/api/v1/crates/levenshtein

### 2.9 Frontend stack (Nov 2026)

- **React 19.x** (current major; exact latest patch UNVERIFIED in this pass), **Vite 7.x** (current major), **Tailwind CSS 4.3.5** (2026-11-06) — use the `@tailwindcss/vite` plugin, no config file, CSS-first `@theme` tokens.
- **shadcn/ui:** as of Feb 2026 uses the unified `radix-ui` package; **as of July 2026 shadcn defaults new projects to Base UI**, with Radix still supported (`shadcn init -b radix`). For a Tauri app either works; if you want the familiar Radix primitives, pass the radix flag explicitly. Flag this drift in the ADR.
- **TanStack Query 5.107.x** (v5 API stable), **TanStack Router 1.x** with TanStack Start 1.0 stable since Oct 2026 (Start not needed for Tauri; Router is fine for SPA mode). File-based or code-based routing both supported.
- **i18next 27.1.4** + `react-i18next` — **v27 is a breaking major** (changed defaults; migration guide v26→v27) — fine for a greenfield app; set up `i18next` + `i18next-resources-to-backend` with EN+VI JSON namespaces; ICU via `i18next-icu` if needed.
- **Tauri layout:** standard `create-tauri-app` shape: repo root = frontend (Vite `src/`, `index.html`), `src-tauri/` (Rust crate, `tauri.conf.json`, `capabilities/`). For a monorepo with a Rust core: `src-tauri/src/main.rs` thin shell + workspace member `core/` (`difflens-core` crate) holding parsing/diff/embeddings logic, so the core is unit-testable headless (`cargo test` without a GUI). pnpm for the frontend; `pnpm tauri dev/build` scripts. CI: `tauri-apps/tauri-action` matrix over `[macos-latest, ubuntu-22.04/24.04, windows-latest]` with pnpm + Rust caches; add `jlumbroso/free-disk-space` on Linux.

**Sources:** https://v2.tauri.app/start/, npm registry data cited in search results (Tailwind 4.3.5, TanStack Query 5.107.1, i18next 27.1.4), shadcn/ui changelog (Feb 2026 unified radix-ui; Jul 2026 Base UI default), https://tanstack.com (Start 1.0 stable Oct 2026)

### 2.10 Licensing audit

| Component | License | MIT-app compatible? | Notes / required actions |
|---|---|---|---|
| Tauri 2 (tauri, plugins, CLI, tauri-action) | MIT OR Apache-2.0 | Yes | Ship license text in about screen |
| PDFium (chromium/pdfium) | **BSD-3-Clause-style** | Yes | Include copyright notice; not copyleft |
| bblanchon/pdfium-binaries | MIT (repo) | Yes | Binaries distribute PDFium under BSD-3 — same notice requirement |
| pdfium-render | MIT OR Apache-2.0 | Yes | — |
| zip (crate) | MIT | Yes | — |
| roxmltree | MIT OR Apache-2.0 | Yes | — |
| calamine | MIT | Yes | — |
| similar | **Apache-2.0** | Yes | Include NOTICE/attribution (Apache requires it) |
| diff-match-patch-rs | MIT OR Apache-2.0 | Yes | — |
| strsim / levenshtein | MIT | Yes | — |
| ort | MIT OR Apache-2.0 | Yes | ONNX Runtime itself is **MIT** (Microsoft) — compatible |
| tokenizers | Apache-2.0 | Yes | NOTICE file |
| intfloat/multilingual-e5-small (+ Xenova export) | **MIT** | Yes | Verified via HF API |
| BAAI/bge-m3 (+ community ONNX exports) | **MIT** (base model) | Yes | Community exports inherit upstream MIT; export scripts (aapot) — check per-repo license UNVERIFIED |
| llama.cpp (incl. llama-server) | **MIT** | Yes | — |
| Qwen3-4B-Instruct-2507 / Qwen2.5-3B (GGUF) | **Apache-2.0** | Yes | Qwen name usage policy exists but is trademark-level, not license-level |
| Llama-3.2-3B-Instruct | **Llama 3.2 Community License** (custom) | **Conditional** | Custom acceptable-use + branding (Llama mark) restrictions; redistribution permitted but it is not MIT/Apache — keep as user-supplied file, not bundled |
| Tesseract CLI / traineddata | Apache-2.0 | Yes | Optional external dependency, not bundled |
| PaddleOCR-class models (if v2) | Code Apache-2.0; per-model weights **UNVERIFIED** | Likely yes | Audit before v2 |
| ONNX model files (any .onnx we download) | Model license governs | n/a | Record model+commit+license in a manifest file bundled with the app |

**AGPL/GPL traps found: none in the chosen stack.** Avoid: anything AGPL (none selected), Llama Community License for *bundled* artifacts (allowed to reference/download at runtime, but don't bundle), and note that "GPL-adjacent" tools like Tesseract are Apache-2.0 since v4 — safe.

---

## 3. Risks

1. **ort is a release candidate (2.0.0-rc.15) and has been for ~2 years.** Mitigation: pin exact version, encapsulate behind a `embed` module trait, evaluate `candle`/`tract` as fallback (both Rust-native, MIT/Apache-class) if rc blocking bugs appear on a target OS.
2. **Community ONNX exports for bge-m3 and e5-small are unofficial** (Xenova/aapot/gpahal). Mitigation: pin commit hashes, re-export ourselves with optimum/paddle2onnx scripts as backup, checksum-verify at download time.
3. **Disk pressure on the build VM (9.5 GB free):** a Tauri `target/` alone can hit 10–20 GB; `ort` downloads ORT binaries at build time, llama.cpp sidecar binaries add ~100 MB across targets. Mitigation: release builds only in CI; local dev = debug build for Linux only + `cargo clean` cadence; move `CARGO_TARGET_DIR` to a bigger mount if available.
4. **OCR scope risk:** PaddleOCR-class OCR via ort has no turnkey Rust path (DBNet polygon postproc must be hand-ported; Vietnamese rec model quality UNVERIFIED). Mitigation: v1 = text-layer + optional Tesseract CLI; keep OCR as v2 behind a spike.
5. **Distribution friction:** unsigned macOS builds hit Gatekeeper and unsigned Windows builds hit SmartScreen; updater requires generated signing keys (free but must be wired from day 1 if you want updates). Mitigation: generate keys at project init, document open instructions, budget for Apple Developer ID ($99/yr) only if/when the project wants it.
6. **Version drift:** i18next 27 breaking major; shadcn/ui default flipped to Base UI (Jul 2026); zip crate at 9.7.0 moves fast; Tauri 3.0 alphas appearing. Mitigation: lockfile + renovate-style updates; record chosen versions in ADRs.
7. **Performance uncertainty on 3-core CPU:** bge-m3 (568M) may be too slow for large documents on weak machines. Mitigation: e5-small default, bge-m3 opt-in with progress UI + cancel, int8 variants, length-sorted batching, cap paragraph count per analysis.

**UNVERIFIED items (explicit):** exact `tauri-action` release tag; ort's exact supported ONNX Runtime version floor (badge says 1.31.0; multiversion range 1.17–1.24 from secondary source); React/Vite exact latest patch numbers; e5-small/bge-m3 3-core latency figures (estimates only); Vietnamese PaddleOCR-class rec quality benchmarks; per-file licenses of community ONNX export repos (aapot/gpahal scripts); PaddleOCR model weight licenses; docx-rs reader depth for tracked changes (claims support in README); exact zip filenames inside Xenova exports (tokenizer.json presence assumed from Transformers.js convention).
