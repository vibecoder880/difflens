# DiffLens

[![CI](https://github.com/vibecoder880/difflens/actions/workflows/ci.yml/badge.svg)](https://github.com/vibecoder880/difflens/actions/workflows/ci.yml)
[![Release](https://github.com/vibecoder880/difflens/actions/workflows/release.yml/badge.svg)](https://github.com/vibecoder880/difflens/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

📚 **Documentation:** full docs at <https://vibecoder880.github.io/difflens/> ·
Wiki: <https://github.com/vibecoder880/difflens/wiki>

**Local-first, cross-platform document diffing with semantic alignment.**

DiffLens compares two documents — PDF, DOCX, TXT, XLSX, PPTX — entirely on
your machine. No accounts, no telemetry, no cloud: your files never leave
your device.

## Why DiffLens

- **Semantic alignment** — paragraphs are matched by meaning (local
  multilingual embeddings), not just position, so moved clauses are detected
  instead of flagged as mass delete+insert noise.
- **Deterministic intelligence** — money / date / percent / duration changes
  are extracted with plain rules; every flagged change shows *which rule*
  fired and the before/after values. Same input → same output, always.
  Auditable by design, no hallucination.
- **Optional local LLM** — want a natural-language explanation of a change?
  Install a small model (Qwen3-4B-Instruct, ~2.4 GB) and DiffLens runs it on
  your machine via llama.cpp. Nothing is sent anywhere.
- **Audit trail** — severity bands (info → critical) come from rule evidence,
  not from a black box.

## Features walkthrough

**1. Home — pick two documents.** Choose a PDF, DOCX, TXT, XLSX or PPTX pair
via the native file picker, adjust comparison options (semantic alignment on
by default; OCR fallback toggle for scanned PDFs), and start the compare. A
progress card shows the pipeline stages (extract → diff → align → heuristic
enrich) with cancel support.

**2. Diff report — the result workspace.** A severity-banded change list
(info → critical, each entry tagged with the rule that fired and the
before/after values) alongside a side-by-side document view with inline
added/removed highlighting. Clicking a change opens the detail drawer with
the full audit trail: extraction provenance, alignment decision, matched
rule evidence. From here you can request a local-LLM explanation for any
change, and export the whole report as HTML or Markdown.

**3. Downloads — explicit, checksum-verified components.** The only screen
that ever touches the network. One click fetches the multilingual-e5-small
embedding model (~118 MB, required), the pinned `llama-server` sidecar and an
optional Qwen GGUF explanation model (~0.6–2.4 GB) — every artifact is
SHA-256-verified against a pinned manifest before use, and downloads are
resumable and cancellable.

**4. Settings — tune the engine.** Switch UI language (English / Tiếng
Việt), pick the alignment embedding strength, choose the LLM model from the
manifest (with size/RAM guidance shown before download), and enable the
experimental OCR path for scanned PDFs. Recent comparisons are listed on the
home screen, backed by a local SQLite session store — history stays on your
disk and can be re-opened or deleted.

> **Screenshots**: not shipped in-repo yet — the app requires a graphical
> environment to run and the docs were finalized on a headless CI machine.
> Grab a release build and see for yourself; the screens above describe
> exactly what you'll find.

## Install

Grab the latest installer for your platform from
[GitHub Releases](https://github.com/vibecoder880/difflens/releases):

| Platform | Artifacts | Notes |
|---|---|---|
| Windows 10/11 x64 | `.msi`, `.exe` (NSIS) | SmartScreen shows an unsigned-installer warning: click **More info → Run anyway** |
| macOS 14+ Apple Silicon | `.dmg` (aarch64) | Unsigned build: after first launch attempt, **right-click → Open**, then confirm. Intel Macs are not supported in v1 |
| Linux x64 | `.AppImage`, `.deb` | AppImage needs FUSE; the `.deb` installs system-wide |

The installers are not OS-code-signed (no Apple Developer ID / Authenticode
certificate for v1). The build pipeline itself runs entirely in GitHub
Actions from clean checkouts, and every artifact URL is published on the
release page. Verify integrity with the SHA-256 checksums published in the
release notes.

## Building from source

Prerequisites: [rustup](https://rustup.rs) (stable), Node.js 24 + npm, and
on Linux the Tauri system libraries:

```bash
# Linux system deps (same line CI uses)
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config

git clone https://github.com/vibecoder880/difflens
cd difflens
npm ci            # installs frontend deps + @tauri-apps/cli
npm run tauri dev # run the desktop app with hot reload
```

Useful commands (all exercised in CI):

```bash
cargo fmt --check -p difflens-core              # formatting gate
cargo clippy -p difflens-core --all-targets -- -D warnings
cargo test -p difflens-core                     # core test suite (deterministic fake embedder)
npm test                                        # vitest + EN/VI i18n parity check
npm run build                                   # frontend typecheck + production build
npm run tauri build                             # full release bundle (per-OS installers)
```

Some test tiers are `#[ignore]`d locally because they need real binaries;
CI runs them on every push: PDF extraction (pinned PDFium via
`scripts/fetch-pdfium.sh`), real e5 embeddings (`scripts/fetch-models.sh`
+ the `embeddings-ort` feature), OCR (`tesseract-ocr-vie`), and a full LLM
roundtrip against the pinned llama-server + a Qwen3-0.6B smoke model.

## Architecture

```
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

The Rust core is a plain library (no UI framework ties): documents are
extracted to normalized blocks, diffed with Myers/Patience, paragraphs are
re-matched semantically with local e5-small embeddings, rule-based heuristics
band severity, and a JSON report with a full audit trail feeds both the UI
and the HTML/Markdown exporters. The Tauri shell is intentionally thin (IPC
commands + session persistence), and the LLM explanation path runs against a
loopback-only llama-server sidecar — it can explain a change, but it can
never alter the severity or the diff result.

## Privacy

DiffLens is 100% local. Your documents are parsed, diffed and scored on your
machine. The **only** network activity DiffLens can perform is downloading
explicitly-requested components through the Downloads manager (embedding
model, LLM sidecar, optional models), from URLs pinned and checksum-verified
in `resources/manifests/llm-manifest.json`. There is no telemetry, no crash
reporting, no analytics — enforced by
[`scripts/audit-egress.sh`](scripts/audit-egress.sh), which runs as a CI gate
on every push and fails if any HTTP client code appears outside
`core/src/download/` (the LLM client is additionally restricted to loopback).

## Supported platforms

| OS | Status | Artifacts |
|---|---|---|
| Windows 10/11 x64 | supported | `.msi`, NSIS `.exe` |
| macOS 14+ (Apple Silicon) | supported | `.dmg` (aarch64) |
| macOS Intel | not supported in v1 | — |
| Linux x64 | supported | `.AppImage`, `.deb` |

## Experimental OCR

Scanned PDFs (no text layer) can be routed through an optional OCR path that
shells out to a locally-installed Tesseract CLI (Apache-2.0; DiffLens never
bundles it — install `tesseract-ocr` + `tesseract-ocr-vie` yourself).
**Quality is honestly mediocre**: expect diacritic errors on Vietnamese and
low-resolution scans; OCR output is marked in the audit trail so you can see
exactly which blocks came from OCR. A native (ONNX-based) OCR engine is a
considered v2 improvement.

## FAQ

- **Are the AI models bundled in the installer?** No. The installers are
  small; the embedding model (~118 MB), `llama-server` and any GGUF model are
  downloaded on demand through the Downloads manager, SHA-256-verified
  against pinned manifests, and stored in your user data directory.
- **Why multilingual-e5-small?** 118M params, 384-dim, strong Vietnamese
  quality, MIT-licensed, and fast enough on modest hardware; a larger
  `bge-m3` option is a future tuning knob, not a v1 default.
- **Is the severity rating AI-generated?** No — severity bands come from
  deterministic rules (money/date/percent/duration deltas, coverage change).
  The same two documents always produce the same report. The optional LLM
  can *explain* a change in natural language but cannot change its severity.

## Status & docs

| Phase | Content | State |
|-------|---------|-------|
| 01 | Toolchain, workspace, CI/CD | ✅ |
| 02 | Extraction (PDF/DOCX/TXT/XLSX/PPTX) | ✅ |
| 03 | Diff + 3-pass semantic alignment | ✅ |
| 04 | Heuristic intelligence | ✅ |
| 05 | llama-server sidecar + explanations | ✅ |
| 06 | OCR (experimental) | ✅ |
| 07 | Frontend (React/Tailwind/i18n) | ✅ |
| 08 | Persistence + report export | ✅ |
| 09 | Release engineering + docs | ✅ |

Release acceptance evidence: [`scripts/release-checklist.md`](scripts/release-checklist.md).

## Documentation

| Document | Contents |
|---|---|
| [Docs site](https://vibecoder880.github.io/difflens/) (mdBook) | full manual: intro, install, quickstart, user guide, how-it-works, AI setup, OCR, privacy, architecture, building, FAQ, changelog |
| [Wiki: Installation](https://github.com/vibecoder880/difflens/wiki/Installation) | per-OS installers, Gatekeeper/SmartScreen notes, SHA-256 verification |
| [Wiki: User Guide](https://github.com/vibecoder880/difflens/wiki/User-Guide) | the four screens in depth (compare wizard, diff view, downloads, settings) |
| [Wiki: How Diffing Works](https://github.com/vibecoder880/difflens/wiki/How-Diffing-Works) | extraction, 3-pass semantic alignment, deterministic heuristics, severity bands |
| [Wiki: AI Explanations Setup](https://github.com/vibecoder880/difflens/wiki/AI-Explanations-Setup) | llama-server sidecar, model downloads, privacy properties |
| [Wiki: Privacy and Security](https://github.com/vibecoder880/difflens/wiki/Privacy-and-Security) | the local-first threat model and download integrity chain |
| [Wiki: Architecture](https://github.com/vibecoder880/difflens/wiki/Architecture) | workspace layout, tech stack table, Tauri command/event surface |
| [Wiki: Building from Source](https://github.com/vibecoder880/difflens/wiki/Building-from-Source) | toolchain, feature flags, test tiers |
| [Wiki: FAQ](https://github.com/vibecoder880/difflens/wiki/FAQ) | twelve common questions |

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Note the hard rule: **any new
network egress must live in `core/src/download/`** and pass
`scripts/audit-egress.sh`.

## License

[MIT](./LICENSE) — see
[THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md) for bundled components,
runtime-downloaded models, licenses and pinned versions.
