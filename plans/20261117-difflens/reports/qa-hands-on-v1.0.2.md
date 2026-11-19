# QA hands-on validation — v1.0.2 release hardening

- **Date:** 2026-11-19 · **Branch:** `main` @ `2e1ec4c` (PDFium bundling fix)
- **Environment:** Linux VM (Debian/Ubuntu), 3-core i5-3337U (Ivy Bridge), 6 GB RAM, ~1.2 GB disk free
- **Method:** real end-to-end invocations of the pipeline through its public API
  (`pipeline::run`, `extract_*`, `Store`, `report::*`, `download::download_file`,
  `llm::sidecar` + `LlmClient`), documented pairs under `/tmp/dl-qa`, repeatable
  integration tests in `core/tests/qa_pipeline.rs` + `core/tests/qa_loader_chain.rs`.
- **Result: 8/8 matrix rows PASS, 1 real bug found and fixed, 1 environment blocker documented (AVX2).**

## Matrix

| # | Test | Result | Observed values |
|---|------|--------|-----------------|
| 1 | Vietnamese contract pair (DOCX), full pipeline, deterministic | **PASS** | stats `{moved:2, added:1, removed:1, modified:3, equal:11}`; two moves detected as `Moved` (single `Equal` span, sim ≈ 1.0), **not** delete+insert; `DURATION_CHANGED` delta **−0.5** ("trong vòng 30 ngày"→"15 ngày"); `PERCENT_CHANGED` delta **+0.5** (see bug 1); `MONEY_CHANGED` delta **+1.4** (5.000.000₫→12.000.000₫) with severity **High**; audit trail 7 entries, run-twice JSON byte-identical |
| 2 | PDF pair (hand-built PDFs, DejaVuSans embedded, Identity-H + ToUnicode) | **PASS** | pdfium extracts 17 blocks over **3 pages**, Vietnamese diacritics intact (`Thanh toán`, `0,075%`); every paired change carries `old_page`/`new_page` within page bounds; DURATION/PERCENT/MONEY flags fire identically through the PDF path |
| 3 | XLSX pair (raw SpreadsheetML, inlineStr) | **PASS** | all blocks are `Cell { sheet: "Bảng giá", row, col }`; B2 money change `1.500.000₫→2.000.000₫` → `MONEY_CHANGED` delta **+0.333333…**; `REFERENCE_CHANGED` on `Số hợp đồng: 2026/HD-001→002`; 3 modified cells |
| 4 | PPTX pair (raw PresentationML) | **PASS** | `Slide { index }` blocks, page = index+1, title in `anchor`; slide 1 `6 tháng→4 tháng` → `DURATION_CHANGED` **−0.3333…**; slide 2 gained milestone line |
| 5 | Markdown pair, word-level spans | **PASS** | spans reconstruct both sides exactly; delete/insert cover only the changed words ("30 "→"45 ", ≤ 10 chars); ≥ 2 Equal runs, no whole-document span |
| 6 | Real LLM roundtrip (cached llama-server b12503 + Qwen3-0.6B-Q8_0, VI prompt) | **PASS** | sidecar healthy in **2.06 s** (loopback, ephemeral port, key via env); **76 tokens in 33.7 s (2.3 tok/s)**, `finish_reason=stop`; VI explanation: *"Thay đổi từ 30 ngày sang 15 ngày, và thay đổi mức phạt từ 0,05% đến 0,075%."* — correctly names the payment-term change; JSON parses via `parse_explanation`; clean kill on drop. GGUF checksum verified against manifest pin |
| 6b | Download manager checksum path (real loopback HTTP) | **PASS** | valid sha → file lands at dest; tampered digest → `checksum mismatch` error, `.part` removed |
| 7 | Store + reports roundtrip | **PASS** | tempfile SQLite: insert→list→get; result JSON byte-stable through store roundtrip; HTML parsed with a real parser (stdlib `html.parser`): zero real `<script>` tags, hostile `<script>alert('dl-xss')</script>` present only as escaped text (`&lt;script&gt;…&#x27;…`), content `0,075%` preserved; delta formatting present (`−50%`, `+140%`, `+50%`, `+33.3%` incl. U+2212 minus); markdown: hostile 3-backtick run fenced inside a **4-backtick** fence, fences balanced |
| 8 | PDFium loader chain | **PASS** | subprocess-per-scenario (bindings are per-process): `DIFFLENS_RESOURCE_DIR` fake-app layout → extraction works (the NEW packaged-app path); dir deleted → falls to repo-staged copy → works; `DIFFLENS_PDFIUM_PATH` pointing at a missing file → fails loudly with the packaging-bug error, no silent fallback |

## Bug found and fixed

**`parse_number` misread Vietnamese small decimals as thousands groups**
(`core/src/heuristic/entities.rs`). Rule 3 ("exactly one separator followed by
exactly 3 digits → integer reading") turned `0,075` into **75**. A zero-only
head can never be a thousands group, so `0,075%` parsed as 75.0 while `0,05%`
correctly parsed as 0.05 — the same contract's penalty change
`0,05% → 0,075%` fired `PERCENT_CHANGED` with **delta +1499.0**
("0.05%→75.00% (+74.95 pp)") instead of **+0.5** (+0.03 pp), inflating
severity points from 30 to 45 for this realistic VN-contract value.

- **Fix (smallest correct):** in the Rule 3 branch, when the head is only
  zeros, read the separator as a decimal separator (`0,075` → 0.075);
  non-zero heads keep the conservative integer reading (`990.000` → 990000).
- **Regression tests:** `zero_head_three_digit_tail_is_decimal_not_thousands`
  + `vietnamese_small_percent_contract_value` (delta +0.5 asserted) in
  `entities.rs`; full suite re-run: **241 lib + 60 integration tests pass**
  (incl. all pre-existing money/percent goldens).

## Severity observations (deterministic engine output, DOCX test 1)

| Change | Kind | Severity | Rules |
|--------|------|----------|-------|
| Delivery clause relocated | Moved | Info | — (identical text) |
| Monthly-report clause deleted | Removed | Info | — (no entities/keywords) |
| Warranty clause relocated | Moved | Low | KEYWORD_RISK (warranty) |
| Heading "Điều 4" renamed | Modified | Low | KEYWORD_RISK (penalty+confidentiality) |
| Payment term 30→15 ngày + 0,05%→0,075% | Modified | **Medium** (50 pts) | DURATION_CHANGED −0.5, PERCENT_CHANGED +0.5 |
| Penalty 5.000.000₫→12.000.000₫ | Modified | **High** (65 pts) | MONEY_CHANGED +1.4 |
| New confidentiality clause | Added | Medium (35 pts) | ENTITY_ADDED ("3 năm"), KEYWORD_RISK (termination+confidentiality) |

Note: `CLAUSE_RENAMED` never fires for DOCX — the flag requires differing
block `anchor`s, and the DOCX extractor only sets anchors for tracked-change
deletions (plain heading renames pair via the "Điều N" clause anchor but carry
`anchor: None`). Alignment is unaffected; the rename surfaces as a Modified
heading with KEYWORD_RISK. Consider deriving clause anchors from heading text
in a future pass if rename attribution matters.

## Honest limitations (what could NOT be tested on this VM)

1. **Real e5-small embeddings — blocked by the CPU, not the code.** The VM's
   i5-3337U has **no AVX2**; ort's prebuilt ONNX Runtime SIGILLs at session
   creation (warning printed, then `signal 4`). With `--features
   embeddings-ort` the process dies before the first embed call, so alignment
   here ran on the deterministic fake embedder (lexical-overlap cosine). All
   fixture assertions were designed to hold under both embedders (moves are
   anchored by unique-text anchors). Real-e5 alignment is CI-covered
   (`ci.yml` embeddings job runs the `#[ignore]` gold gate on AVX2 runners).
   **Carry-over:** if phase-09 ever enables `embeddings-ort` in release
   builds, machines without AVX2 (pre-2013 CPUs) would crash the app — the
   shipped release does NOT enable the feature today, so the bundled-by-`--bundle`
   e5 model is currently unused payload; README's "semantic alignment" claim
   in a shipped build degrades to positional/anchor matching
   (`embeddings_used = false`).
2. **LLM download-manager resume skipped** (kill-curl-mid-download is
   invasive, per plan). The resume code path stays CI/untested here; the
   checksum-verification path WAS exercised for real (639 MB sha256 match +
   tampered-digest failure).
3. **GUI layer** (webkit2gtk missing — `cargo check -p difflens` cannot link
   `gio-sys`/`gtk` here): the Tauri shell (jobs, progress events, dialogs)
   was not exercised; `commands.rs` headless tests cover `execute_job`,
   persistence and `write_report` in CI instead.
4. **OCR** (no tesseract + vie traineddata locally): CI-covered by
   `ocr_tests` + fixture generation; `ocr_used` stayed false in all runs.
5. **macOS/Windows binaries:** cannot run on this VM. Verify after the
   release build via CI artifact inspection: (a) the existing release.yml
   step asserting `pdfium.dll`/`libpdfium.so`/`libpdfium.dylib` inside each
   bundle (the v1.0.0/1.0.1 regression); (b) `DIFFLENS_RESOURCE_DIR` +
   `<exe_dir>/pdfium/` resolution on Windows (loader test 8 covers Linux
   only); (c) installer contents include the llama-server download UI assets
   but no GGUF (by design, downloaded on demand).
6. **Disk:** ~1.2 GB free; the 639 MB test GGUF was reused from
   `/tmp/llm-verify/model/` (checksum-verified) instead of re-downloading.
   `qa_llm_real_roundtrip_vi` falls back to a real `download_file` download
   if no cached GGUF exists.

## How to re-run

```sh
python3 -I /tmp/dl-qa/gen_docs.py          # regenerate document pairs
cargo test -p difflens-core --test qa_pipeline -- --test-threads=1
cargo test -p difflens-core --test qa_loader_chain
cargo test -p difflens-core --features embeddings-ort \
  --test qa_pipeline qa_llm_real_roundtrip_vi -- --ignored --nocapture
python3 -I /tmp/dl-qa/check_report.py      # external HTML/MD report audit
```

(`--features embeddings-ort` requires AVX2 locally; CI runners are fine.
openssl-sys needs `pkg-config`, or
`OPENSSL_NO_PKG_CONFIG=1 OPENSSL_LIB_DIR=/usr/lib/x86_64-linux-gnu OPENSSL_INCLUDE_DIR=/usr/include`.)

## Files left in tree (uncommitted, for orchestrator review)

- `core/src/heuristic/entities.rs` — parse_number zero-head fix + 2 regression tests (MODIFIED)
- `core/tests/qa_pipeline.rs` — tests 1–7 + download checksum test + ignored LLM roundtrip (NEW)
- `core/tests/qa_loader_chain.rs` — test 8, subprocess-per-scenario loader chain (NEW)
- Scratch artifacts outside the repo: `/tmp/dl-qa/*` (documents, generator, report checker)
