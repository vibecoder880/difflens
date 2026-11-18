---
title: "OCR ONNX spike — research note only, NO-GO for v1"
status: recorded
date: 2026-11-18
phase: 06-ocr
---

# ONNX OCR Spike — Research Note (NO-GO for v1)

**Resolution S3 (red-team review):** the ONNX OCR spike ships as a research
note only — no spike code, no cargo feature in v1. This file is that note.
Productized OCR in v1 is the user-installed Tesseract CLI path
(`core/src/ocr/`, experimental, honest labeling).

## Decision

**NO-GO for v1** (default per research §2.7, confirmed at phase-06 time).
PaddleOCR-class OCR via `ort` is deferred to v2 planning and gated behind the
acceptance gate below.

## NO-GO rationale

1. **DBNet post-processing port cost.** A `ort`-based pipeline needs three
   models (DBNet det ~4.7 MB, angle cls ~1.4 MB, CRNN/SVTR rec ~10–15 MB) plus
   preprocessing, the DBNet bitmap post-processing (binarize → contours →
   polygon unclip, ≈300 lines of C++/Python to port — no mature pure-Rust
   crate), CTC decoding, and reading-order line grouping. Community
   assessment stands: multi-week effort with real risk concentrated in the
   postprocessing and reading-order stages (research §2.7).
2. **Unverified Vietnamese quality of community ONNX models.** Candidates
   (`monkt/paddleocr-onnx`, RapidOCR PP-OCRv4/v5/v6, vi-specific rec models on
   HF) list Vietnamese support, but no independent CER benchmarks on real
   Vietnamese scanned documents were found (research §2.7, still UNVERIFIED at
   phase-06 close). Shipping a v1 quality risk behind an unverified benchmark
   violates the honest-labeling contract.
3. **Tesseract CER 18.2% vs PaddleOCR-class ~4.5%.** The documented quality
   gap (research: Vietnamese diacritics errors on real scans) is real but
   bounded: v1 scopes OCR to scanned-vs-digital and scanned-view comparisons,
   surfaces the experimental label and mean-confidence metadata, and warns on
   scanned-vs-scanned diffs (alignment quality is hopeless without strong OCR
   on both sides). Tesseract is detected, not shipped — zero bundling,
   licensing, and size burden for a mediocre-but-known v1 quality bar. The
   **4.5%-class accuracy is the v2 target**, not a v1 requirement.

## Acceptance gate for the v2 revisit

ONNX OCR work is scheduled only when **all** of the following pass:

- **Benchmark harness exists:** a fixture corpus of Vietnamese scanned
  documents (≥50 pages spanning printed contracts, invoices, low-DPI scans,
  skew/noise) with ground-truth transcripts, plus a CER runner computing
  per-page and aggregate character error rate against it.
- **CER threshold:** a candidate ONNX stack (det + cls + rec, one of the
  community model sets) measures **CER < 10% on the VN fixture corpus** —
  i.e. demonstrably better than Tesseract's 18.2% by a margin that justifies
  the port — measured end-to-end (not per-component paper numbers).
- **Postprocessing cost re-estimated** against a reference implementation
  after the benchmark (polygon unclip + reading order), with a deterministic
  Rust port plan and no `unsafe` beyond `ort` itself.
- **Licensing audit:** per-file license check of the chosen model weights
  (research §2.10: code Apache-2.0, weights Apache-2.0 per family but
  UNVERIFIED per file).
- **Bundle budget:** total model payload ≤ 25 MB and startup/runtime memory
  within the 3-core/6 GB dev-VM envelope used throughout the phases.

## Current v1 state (for context)

- Tesseract path shipped as `core/src/ocr/` (detect / rasterize / engine /
  labels), experimental warnings + `anchor: "ocr"` provenance on every OCR
  block, 50-page cap, per-page timeout, graceful degradation when tesseract or
  `vie` is missing.
- Measured on the committed clean-rendered fixture (`vietnamese_scanned.png`,
  CI core job): Vietnamese contract vocabulary ("Thanh toán", "Hợp đồng",
  "bồi thường") recovers verbatim with mean confidence ≈95%. The 18.2% CER
  figure applies to *real-world scans* (noise, skew, low DPI), which the
  experimental label covers honestly.

## Disposition

- v1: this note only; no spike code, no `ort` OCR feature, no model downloads.
- v2: revisit behind the gate above; if it fails, re-run the gate after the
  next community model generation (PP-OCRv6-family) or accept Tesseract as the
  permanent, honestly-labeled baseline.
