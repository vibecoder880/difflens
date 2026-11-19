# User Guide

Four screens: Compare wizard (home), Diff view, Downloads, Settings. The GUI needs a display to run, so no screenshots exist yet — layouts are described in words (plus one ASCII sketch).

## Compare wizard (Home)

- **Two file slots** with native file-picker buttons; drag-and-drop fills empty slots in order (one drop per slot, or two files at once fill both).
- **Format badges** auto-detected from the extension — spot a wrong pick before running.
- **Same-file validation**: identical files in both slots disable Compare with a hint.
- **OCR toggle** (default auto) — see [OCR](ocr.md).
- **Progress card** — `diff://progress` events drive stage + percent: `extract → ocr (conditional) → align → diff → enrich → done/failed/cancelled`, with a working Cancel.
- **Recent comparisons** — SQLite-backed history; click to re-open, delete to remove.

## Diff view

```
┌─ Diff report ─────────────────────────────────────────────────┐
│ [added 2] [removed 1] [modified 4] [equal 31]  e5 ✓ embeddings│
│──────────────┬────────────────────────────────┬───────────────┤
│ FILTERS      │  OLD (v1)          │ NEW (v2)  │ CHANGE LIST   │
│ kind/severity│  Payment within 30 │ …15 days  │ ● HIGH  MONEY │
│ chips+counts │  (unchanged runs   │           │   −50%        │
│              │   collapsed)       │           │ ● MED   DATE  │
│              │  inline: [30]{15}  │           │ ○ LOW   entity│
│──────────────┴────────────────────────────────┴───────────────┤
│ DETAIL DRAWER: flags (old→new) · audit trail ("why this       │
│ severity") · [Explain with AI ▸ stream ▐ stop] · [Export ▾]   │
└────────────────────────────────────────────────────────────────┘
```

- **Stats chips**: added/removed/modified/equal counts, an embeddings indicator (real e5 vs deterministic fallback — mirrored from `embeddings_used`), OCR flag when used.
- **Virtualized side-by-side rows** (TanStack Virtual) stay smooth on large documents; inline spans color the exact words that changed; moved rows show an arrow with positional delta; unchanged block runs collapse.
- **Change list filters**: kind and severity chips with live counts; clicking a change reveals it and opens the drawer.
- **Detail drawer**: heuristic flags with old → new values; the audit trail explaining severity (rule points + keyword uplift, alignment source with similarity); **Explain with AI** (streaming tokens + Stop, once a model is installed); **Export** menu (HTML / Markdown via native save dialog).

## Downloads

The only screen that touches the network:

- **llama-server install card** — pinned `b12503`, per-OS, with progress streamed on `llm://download`; a `llama-server` already on `PATH` is detected as a fallback.
- **Model cards** — Qwen3-4B (primary) and Qwen2.5-3B (fallback) with sizes, licenses, minimum-RAM guidance and a **SHA-256 verified** badge; resumable (`.part`) and cancellable.
- **OCR availability card** — detected Tesseract binary + languages.

## Settings

- **Theme** — light / dark / system.
- **Language** — English or Tiếng Việt (persisted; both locales parity-tested).
- **OCR preference** — off disables the OCR path entirely (no detection, no warnings); on = auto mode (scanned PDFs only, when Tesseract + `vie` are available).
- **About** — version and license.

## History

Every finished comparison persists (fire-and-forget) to `difflens.sqlite3` in the app-data directory: paths, format, stats, embeddings/OCR flags and the full result JSON — a re-opened session is the identical report. See [Privacy and Security](privacy-security.md) for what that means.
