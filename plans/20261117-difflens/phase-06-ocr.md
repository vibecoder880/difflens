---
title: "Phase 6: OCR path (Tesseract detection, rasterize→OCR, experimental flag)"
status: todo
priority: P3
effort: 16h
---

# Phase 6: OCR Path (Experimental, Tesseract CLI)

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.7 (OCR verdict: v1 = Tesseract CLI, ONNX OCR NO-GO behind spike; VN CER ~18%), Phase 2 `ScannedPdfDetected` warning, Phase 3 embeddings.

## Overview

**Priority:** P3 · **Status:** pending · **Effort:** ~16h
Add the optional OCR path for scanned PDFs: detect a user-installed `tesseract` CLI (with `vi` traineddata check), rasterize PDF pages via pdfium-render, run Tesseract per page, and feed resulting text blocks back through the normal extraction→diff pipeline. Shipped as an experimental feature flag with honest UI labeling (Phase 7 renders the label). ONNX OCR is evaluated as a research note only (no product code, no cargo feature in v1) — default NO-GO per research, scheduled for v2 planning.

## Key Insights

- Zero bundling burden: Tesseract is detected, not shipped; missing binary/language just disables the feature with a helpful message (install pointers per OS).
- Scope v1 to scanned-vs-digital and scanned-view comparisons; scanned-vs-scanned alignment quality is hopeless without strong OCR on both sides (research §2.7) — the UI should warn, not silently produce garbage.
- Vietnamese quality is mediocre-but-known (~CER 18%) — the "experimental" label is a contract requirement, not optional polish.
- The pdfium render API (Phase 2 loader reuse) at 300 DPI is the only new rendering work; OCR output reuses the block model so downstream diff/alignment/heuristics work unchanged.

## Requirements

- [ ] Tesseract detection: PATH + known install locations, version parse, `--list-langs` check for `vie`.
- [ ] Rasterize→OCR→blocks pipeline returning an `ExtractedDoc` with per-page blocks and OCR metadata.
- [ ] Feature flag `ocr_enabled` (default off) + experimental labeling surfaced to UI; page cap (50) per job.
- [ ] ONNX OCR evaluated as a research note only (no product code, no feature in v1); decision recorded with measured CER; default NO-GO, scheduled for v2 planning.
- [ ] `cargo test -p difflens-core ocr` green (detect tests skip gracefully when Tesseract absent).

## Architecture

`core/src/ocr/detect.rs` returns `OcrAvailability {binary: Option<PathBuf>, version, has_vie}`. `core/src/ocr/run.rs`: given a PDF path + options, renders pages to temp PNGs via the Phase 2 pdfium loader, spawns `tesseract <png> stdout -l vie+eng --psm 3` (fallback `-l eng` with warning if `vie` missing), captures text, splits to paragraph blocks with `page_no` + `style.ocr = true`, returns `ExtractedDoc {format: PdfOcr, warnings}`. The ONNX OCR evaluation (DBNet+CRNN CER bench) is a research note in `plans/20261117-difflens/reports/` — no product code, no cargo feature; default NO-GO per research, v2 scope.

## Related Code Files

- `core/src/ocr/mod.rs` (create — module + public API + feature flag wiring)
- `core/src/ocr/detect.rs` (create — binary/lang detection)
- `core/src/ocr/run.rs` (create — rasterize + spawn + block assembly)
- `core/src/model.rs` (modify — add `Format::PdfOcr` and `StyleHints.ocr`; Phase 7's `src/lib/types.ts` mirror must be updated when this lands)
- `core/src/extract/mod.rs` (modify — accept OCR-produced ExtractedDoc in pipeline entry)
- `src-tauri/src/commands.rs` (modify — add `ocr_status`, plumb OCR option into the Phase 3 `run_diff`)
- `core/tests/ocr_tests.rs` (create)
- `scripts/fixtures/generate.mjs` (modify — emit a rasterized scanned-like PDF fixture via pdf-lib + `@napi-rs/canvas` rasterization or committed pre-rendered PNG assets; pdf-lib alone cannot rasterize text)
- `plans/20261117-difflens/reports/ocr-onnx-spike-research.md` (create — research note only, no product code)

## Implementation Steps

1. `detect.rs`: locate binary via PATH then OS-specific known paths (Windows `C:\Program Files\Tesseract-OCR\`, macOS `/opt/homebrew/bin` + `/usr/local/bin`); parse `tesseract --version`; `--list-langs` grep `vie`; return availability struct; unit tests skip when absent.
2. `run.rs`: page loop with pdfium render (300 DPI default, configurable; max 50 pages — beyond cap emit warning and stop); write PNGs to a per-job temp dir; spawn Tesseract concurrently bounded (2 jobs) with stdout capture; assemble paragraph blocks (split on blank lines) preserving page order; attach `ExtractionWarning::OcrExperimental` and per-page warnings for empty results.
3. Language selection: prefer `vie+eng` when `vie` available; else `eng` with warning instructing how to install `tesseract-ocr-vie` per OS.
4. Pipeline integration: `extract/mod.rs` gains `extract_pdf_with_ocr(path, ocr: OcrOptions)` used when the user enables OCR or when `ScannedPdfDetected` fires and OCR is enabled; result flows into the normal diff/alignment path unchanged (embeddings handle OCR noise via similarity, not exactness).
5. Tauri plumbing: `ocr_status` command (availability + lang info for UI guidance); the Phase 3 `run_diff` options gain `ocr: OcrOptions {enabled, dpi, lang}`; `diff://progress` events include OCR stage with per-page granularity.
6. ONNX OCR research note (no product code, no cargo feature): bench DBNet+CRNN ONNX on fixture scans offline, compute CER vs fixture ground truth; write the decision note to `plans/20261117-difflens/reports/ocr-onnx-spike-research.md`; only if CER < 10% schedule v2 work (not v1 scope — default NO-GO recorded).
7. Tests: detection unit tests (mock PATH); rasterize+OCR e2e `#[ignore]` test requiring `tesseract` + `vie` (CI ubuntu job apt-installs `tesseract-ocr tesseract-ocr-vie` and runs it); page-cap and lang-fallback warning tests.

## Todo

- [ ] detect.rs with graceful skip tests
- [ ] run.rs rasterize→OCR→blocks with caps + warnings
- [ ] `vie+eng`/`eng` language fallback logic
- [ ] Pipeline + Tauri command integration (ocr_status, options)
- [ ] Scanned-like PDF fixture generated (canvas/PNG rasterization)
- [ ] ONNX OCR research note in reports/ (no product code; default NO-GO recorded, v2 scope)
- [ ] `cargo test -p difflens-core ocr` green

## Success Criteria

- `cargo test -p difflens-core ocr` exits 0 on a machine without Tesseract (skips) and with it installed (e2e passes).
- With Tesseract + `vie` installed, diffing the scanned-like fixture against its digital counterpart produces OCR-sourced blocks that align (Moved/similar pairs) and carry the experimental warning.
- `ocr_status` returns accurate availability JSON consumed by Phase 7 settings UI.
- ONNX OCR decision note (research only, no product code, measured CER) recorded in `plans/20261117-difflens/reports/ocr-onnx-spike-research.md`; default NO-GO scheduled for v2 planning.

## Risk Assessment

- Tesseract missing/misconfigured on user machines → detection + per-OS install guidance in UI; feature is opt-in so core diff never depends on it.
- Vietnamese OCR quality below expectations → honest experimental labeling (contract) + scanned-vs-scanned warning; quality improvement is v2 scope.
- Spike scope creep → time-boxed bench only, feature-gated out of default builds and CI.

## Security Considerations

- Tesseract runs on temp-dir files only; temp dir cleaned per job; spawned binary is the user's own installed CLI invoked with fixed argument shapes (no user text in argv).
- OCR adds no network or telemetry.

## Next Steps

Phase 7 surfaces OCR status/options with honest experimental labels and builds the full compare/diff UI on top of Phases 2–5 outputs.
