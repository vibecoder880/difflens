# Quickstart: your first diff

This walkthrough gets from zero to a severity-banded report in a few minutes.

## 1. Pick two documents

Launch DiffLens. The home screen shows two file slots — **Old** and **New**. Click *Browse* in each to pick files with the native dialog, or simply drag both files onto the window (they fill the slots in order). Format badges (PDF/DOCX/XLSX/PPTX/TXT) confirm what was detected; both files must be the same format.

For a first try, two versions of the same contract work best — text PDFs, not scans (OCR is experimental, see [OCR](ocr.md)).

## 2. Compare

Press **Compare**. A progress card tracks the job with stage names and percentages (`extract → ocr → align → diff → enrich → done`); Cancel flags the job between stages. First run downloads the e5 embedding model (~118 MB) through the Downloads manager when prompted — SHA-256-verified, one time.

## 3. Read the report

The diff view has three regions:

- **Change list** — each change with severity band (info → critical), kind (added/removed/modified/moved), and rule tags. Filter chips at the top.
- **Side-by-side view** — aligned rows with inline added/removed coloring; moved rows marked with an arrow; unchanged runs collapsed behind "N unchanged" expanders.
- **Detail drawer** — click any change: old → new values for each flag, the full audit trail (extraction provenance, alignment decision, which rule fired and why the severity scored what it did).

Try a money change: a clause going from "30 days" to "15 days" should show `DURATION_CHANGED` with an explicit **−50%** delta.

## 4. Export

Use **Export** to write a standalone HTML or Markdown report through the native save dialog — same content, hostile-content-escaped, zero JavaScript.

## 5. Optional: AI explanation

On the Downloads screen, install `llama-server` (~65 MB) and a model (Qwen3-4B ~2.4 GB if you have ≥ 8 GB RAM; Qwen2.5-3B ~2.0 GB otherwise). Back in the detail drawer, **Explain with AI** streams a plain-language summary of that one change — generated locally, cancellable, and powerless to change the result. Details: [AI Explanations Setup](ai-setup.md).

## 6. History

The comparison is saved to a local SQLite store automatically; recent sessions are listed on the home screen and can be re-opened or deleted. Everything stays on your disk.
