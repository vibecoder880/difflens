---
title: "Phase 2: Core extraction layer (TXT/PDF/DOCX/XLSX/PPTX → block model)"
status: todo
priority: P1
effort: 40h
---

# Phase 2: Core Extraction Layer

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.2 (pdfium-render + char boxes + bundling), §2.3 (raw DOCX parse decision), §2.4 (calamine, PPTX pattern).

## Overview

**Priority:** P1 · **Status:** pending · **Effort:** ~40h
Build the `difflens-core` extraction layer: turn TXT, PDF, DOCX, XLSX and PPTX files into one unified, ordered block model with normalization, plus a committed fixture corpus and per-format tests. Raw zip+roxmltree parsing for DOCX/PPTX (brainstorm trade-off), pdfium-render with bundled PDFium binaries for PDF, calamine for XLSX. Every extractor degrades gracefully: unsupported/exotic features produce warnings, never failures, and never panics.

## Key Insights

- DOCX fidelity lives in the details: `w:delText` inside `w:del` (not `w:t`), run ordering, `w:hyperlink`, `w:tbl`. Writing our own ~500–800-line parser is the researched-best approach for diff-grade extraction.
- pdfium-render loads `libpdfium.so`/`pdfium.dll`/`libpdfium.dylib` at runtime via `bind_to_library` — bundle bblanchon binaries per OS as Tauri resources; paragraph reconstruction on char boxes is our code (Y-cluster/X-order).
- calamine reads cached cell values + formula strings but never evaluates — diff compares both and flags "formula changed, cached value same".
- A "scanned PDF" signal (pages with no text layer) must be captured here — it feeds Phase 6 OCR and the UI notice.

## Requirements

- [ ] Unified `DocBlock`/`ExtractedDoc` model with kind, page/ordinal position, style hints, raw + normalized text.
- [ ] Extractors for all 5 formats dispatching on magic bytes, with `ExtractionWarning` for degraded cases.
- [ ] Fixture corpus committed under `core/tests/fixtures/` (<100 KB per file) + generator script.
- [ ] Zip-bomb and resource limits enforced; no panic on malformed input.
- [ ] `cargo test -p difflens-core extract` green including fixture-based tests.

## Architecture

`core/src/model.rs` defines `BlockKind {Paragraph, Heading, ListItem, TableCell, SlideText, Row}`, `StyleHints {bold, heading_level, formula, sheet, cell_ref}`, `DocBlock {id, kind, text, normalized_text, page_no, ordinal, style}`, `ExtractedDoc {format, meta, blocks, warnings}`. `core/src/extract/mod.rs` dispatches on magic bytes + `[Content_Types].xml` sniff, returns `Result<ExtractedDoc, ExtractError>`. Normalization (NFC, whitespace collapse, quote/dash unification) stored alongside raw text so Phase 3 diffs raw and aligns normalized. `pdfium_loader.rs` resolves the bundled PDFium library path per OS with a system-library fallback.

## Related Code Files

- `core/src/model.rs` (create — unified block model + normalization)
- `core/src/extract/mod.rs` (create — dispatch + error/warning types)
- `core/src/extract/txt.rs`, `core/src/extract/pdf.rs`, `core/src/extract/docx.rs`, `core/src/extract/xlsx.rs`, `core/src/extract/pptx.rs` (create)
- `core/src/extract/pdfium_loader.rs` (create — runtime binding to bundled PDFium)
- `core/src/extract/normalize.rs` (create — NFC/whitespace/quote normalization)
- `core/tests/extract_tests.rs`, `core/tests/fixtures/*.txt|pdf|docx|xlsx|pptx` (create)
- `scripts/fixtures/generate.mjs` (create — pdf-lib PDFs; devDependency only)
- `scripts/fixtures/templates/docx/*.xml`, `scripts/fixtures/templates/pptx/*.xml` (create — hand-authored OOXML zipped via `zip` CLI)
- `scripts/fetch-pdfium.sh` (create — download pinned bblanchon pdfium-binaries per OS + SHA256 into `src-tauri/resources/pdfium/`, gitignored)
- `.github/workflows/ci.yml` (modify — add PDFium staging step before `cargo test`; see step 11)

## Implementation Steps

1. `model.rs`: define the block model, `ExtractionWarning {code, message, block_hint}`, `ExtractError` (thiserror); implement `normalize()` (unicode NFC via `unicode-normalization`, collapse whitespace, unify curly quotes/dashes) producing `normalized_text` next to raw `text`.
2. `extract/mod.rs`: `extract(path: &Path) -> Result<ExtractedDoc>`; sniff magic bytes (`PK\x03\x04` → container: detect docx/xlsx/pptx by `[Content_Types].xml` + part presence; `%PDF-` → pdf; else text); map extension mismatches to warnings; enforce limits here (max 10k zip entries, max 512 MB decompressed, depth 3).
3. `txt.rs`: BOM detection (UTF-8/UTF-16LE/BE), lossy latin-1 fallback with warning; split paragraphs on blank lines; keep line numbers in `ordinal`.
4. `pdfium_loader.rs`: locate bundled library in `src-tauri/resources/pdfium/<target-triple>/` (resolvable via env override `DIFFLENS_PDFIUM_PATH` for tests), `Pdfium::bind_to_library`, fallback `bind_to_system_library` with warning.
5. `pdf.rs`: per page, iterate `PdfPageText::chars()` with `char_box()`; cluster into lines by Y tolerance, order X-wise; merge lines into paragraphs via indent/spacing/width heuristics; set `page_no`; count zero-text pages and emit `ScannedPdfDetected` warning when >30% of pages have no text layer.
6. `docx.rs`: open zip, parse `word/document.xml` with roxmltree: `w:p` → paragraph blocks from `w:r/w:t` in order; `w:tab`→`\t`, `w:br`→`\n`; `w:hyperlink` inner runs included; `w:tbl/w:tr/w:tc` → TableCell blocks with sheet-like path metadata; `w:ins` rendered accepted, `w:del/w:delText` excluded by default with `TrackedChangesMode` enum stub (Accepted view; toggle wired in Phase 7); headers/footers/footnotes skipped with warning.
7. `xlsx.rs`: calamine `open_workbook_auto`; per visible sheet, rows → Row blocks (cells joined with `\t`, blanks skipped at row ends); cell formula strings kept in `StyleHints.formula`; cap 100k rows with warning.
8. `pptx.rs`: `ppt/presentation.xml` slide ordering; per `ppt/slides/slideN.xml` collect `<a:t>` runs in document order → SlideText blocks; `ppt/notesSlides/` optional with warning when present but skipped.
9. Fixtures: `scripts/fixtures/generate.mjs` (devDependencies `pdf-lib` + `@napi-rs/canvas`) generates multi-page text PDF + one image-only (scanned-like) PDF — text is rasterized to PNG via `@napi-rs/canvas` (or committed pre-rendered PNG assets), since pdf-lib draws vector text only and cannot rasterize; hand-authored OOXML templates + `zip` CLI build a DOCX with tracked changes + table, an XLSX with formulas, a multi-slide PPTX; commit all under `core/tests/fixtures/`.
10. Tests in `core/tests/extract_tests.rs`: per-format expected block sequences (text + kind + page_no); normalization table test; zip-bomb fixture test (small zip, huge logical size → bounded error); graceful-degradation test (DOCX with exotic part → warning + text-only result); no-panic fuzz-lite loop over truncated fixture bytes.
11. CI PDFium staging (keeps "CI green on main" true from Phase 2 onward): add a ci.yml step per runner OS that runs `scripts/fetch-pdfium.sh`, exports `DIFFLENS_PDFIUM_PATH` to the fetched library path, and asserts the pdf fixture tests actually executed (not skipped) on Linux — e.g., fail the job if the pdf loader test did not run.

## Todo

- [ ] model.rs + normalize.rs with unit tests
- [ ] Dispatch + limits + warning plumbing in extract/mod.rs
- [ ] txt, pdf (loader + char-box paragraphs), docx (tracked changes + tables), xlsx (formulas), pptx extractors
- [ ] Fixture corpus committed + generator script
- [ ] `scripts/fetch-pdfium.sh` populates `src-tauri/resources/pdfium/`
- [ ] `cargo test -p difflens-core extract` green

## Success Criteria

- `cargo test -p difflens-core extract` exits 0 (fixture-based tests included).
- `cargo clippy -p difflens-core -- -D warnings` exits 0.
- Extracting each committed fixture yields the expected block count and text assertions encoded in the tests.
- Malformed/truncated inputs return `Err` or warnings — no Rust-level panics across the fuzz-lite loop (PDFium C-layer SIGSEGV/SIGABRT on hostile PDFs is uncatchable in-process and out of scope of this harness — documented as a known limitation below).

## Risk Assessment

- Exotic OOXML (smartart, alternate content) fails parsing → per-part try/parse with `ExtractionWarning` and text-only degradation; fixture corpus grows as failures are found (brainstorm: fails first, degrade gracefully).
- Rotated/complex PDF text layout produces wrong reading order → document limitation via warning; correctness gate only on axis-aligned fixtures.
- PDFium C layer can hard-crash (SIGSEGV/SIGABRT, not a Rust panic) on hostile input → documented known limitation; the fuzz-lite criterion claims "no Rust-level panics" only. Pin/track pdfium-binaries updates as a standing v1.x maintenance note; the bundled PDFium CVE surface is re-checked in Phase 9's security considerations.
- PDFium resource path differs per OS/triple → loader unit test with `DIFFLENS_PDFIUM_PATH` override runs on all CI OSes; ci.yml stages the library first (step 11) so `cargo test --workspace` stays green from this phase on.

## Security Considerations

- Zip limits (entries, decompressed size, depth) enforced before parsing; roxmltree has no DTD/entity expansion by default.
- Files are opened read-only; paths never leave the process except into session metadata (Phase 8).

## Next Steps

Phase 3 consumes `ExtractedDoc` blocks for word/char diff and 3-pass paragraph alignment; it also adds the e5-small model fetch script feeding `src-tauri/resources/models/`.
