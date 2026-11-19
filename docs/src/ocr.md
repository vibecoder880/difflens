# OCR (Experimental)

Scanned PDFs can be pulled through an optional OCR path — labeled **experimental** because quality is honestly mediocre.

## When it triggers

All of the following must hold:

1. The document is a **PDF**.
2. **More than 30% of pages** have an empty text layer (the "scanned PDF suspected" threshold).
3. The Settings **OCR preference** is not switched off (default: auto).
4. A locally installed **Tesseract with the `vie` traineddata** is detected.

Otherwise extraction proceeds without OCR — never fails, never blocks, at most one honest warning.

## Installing the dependency

Tesseract is **detected, not shipped** (Apache-2.0 since v4):

```bash
# Debian / Ubuntu
sudo apt-get install tesseract-ocr tesseract-ocr-vie

# macOS
brew install tesseract && brew install tesseract-lang   # includes vie
```

Windows: the [UB-Mannheim installer](https://github.com/UB-Mannheim/tesseract/wiki), tick **Vietnamese** (`C:\Program Files\Tesseract-OCR` is probed automatically).

Detection checks `PATH` first, then known locations per OS; a binary counts as usable only if `tesseract --list-langs` reports `vie`. The Settings and Downloads screens show the availability card (binary path + languages).

## Execution details

- Textless pages rasterized at **300 DPI** via the pinned PDFium loader.
- Per-page tesseract runs with the `vie+eng` pair under a **hard 90 s timeout**; max **50 pages** per job (capped with a warning).
- TSV output → geometric line reconstruction → ordinary blocks. Downstream diff/align/heuristics work unchanged.

## Honest accuracy caveat

Expect mistakes: diacritic errors on Vietnamese and degradation on low-resolution scans. Community benchmarks place Tesseract's character error rate on Vietnamese text around **~18%**, versus roughly **~4–5%** for modern detector+recognizer ONNX pipelines (PP-OCRv5-class) — approximate, order-of-magnitude numbers, but the gap motivates the considered **v2 improvement**: a native ONNX OCR engine reusing the `ort` runtime already shipped for embeddings, which would also remove the external Tesseract dependency.

Because of that, OCR output is labeled everywhere: every OCR block carries `anchor: "ocr"` provenance in the audit trail; the document gains "N pages extracted via OCR (experimental)" warnings plus **mean confidence**; low-confidence text is flagged, never presented silently as fact.

## Disabling

Settings → OCR preference → **off** (maps to `run_diff`'s `ocr_enabled` flag): no detection, no warnings, no subprocess. On a machine without Tesseract the auto default never activates anything.

## CI verification

The `core` CI job installs `tesseract-ocr` + `tesseract-ocr-vie`, asserts `vie` appears in `--list-langs`, runs the OCR integration tests, and greps the log for `ocr-ci-ok` — silent skipping fails the build.
