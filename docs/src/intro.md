# Introduction

**DiffLens** compares two documents — PDF, DOCX, XLSX, PPTX or TXT — entirely on your machine, and produces a severity-banded change report with a full audit trail. Paragraphs are matched *by meaning* through local multilingual embeddings, so moved clauses are detected as moves rather than delete-plus-insert noise. Money, date, percent and duration changes are extracted by deterministic rules that show which rule fired and the before/after values — the same inputs always yield the same report. An optional local LLM (Qwen via llama.cpp, loopback-only) can explain any change in English or Vietnamese; it can never alter the diff or its severity.

> **At a glance:** 100% local · MIT licensed · 5 formats · English + Tiếng Việt

| | |
|---|---|
| Platforms | Windows 10/11 x64 · macOS 14+ (Apple Silicon) · Linux x64 |
| Current release | 1.0.1 |
| License | MIT ([third-party notices](https://github.com/vibecoder880/difflens/blob/main/THIRD-PARTY-NOTICES.md)) |
| Repository | [vibecoder880/difflens](https://github.com/vibecoder880/difflens) · [Wiki](https://github.com/vibecoder880/difflens/wiki) |

## Why DiffLens

- **Semantic alignment** — e5-small embeddings run locally; a 3-pass aligner (deterministic anchors → Needleman–Wunsch → move repair) keeps contract structure intact.
- **Deterministic intelligence** — plain rules, published point values, banded severity (info → critical). Auditable by design; no hallucination possible in the scoring path.
- **Optional local LLM** — download a small Apache-2.0 model through the in-app manager; explanations stream from a loopback-only sidecar. Nothing is sent anywhere.
- **Honest tooling** — OCR is labeled experimental; every download is checksum-gated; the egress invariant is enforced by CI.

## Feature overview

| Feature | Chapter |
|---|---|
| Installing per OS + SHA-256 verification | [Installation](installation.md) |
| First comparison walkthrough | [Quickstart](quickstart.md) |
| The four screens in depth | [User Guide](user-guide.md) |
| Pipeline: extract → align → diff → enrich | [How Diffing Works](how-it-works.md) |
| Downloading models, privacy properties | [AI Explanations Setup](ai-setup.md) |
| Scanned PDFs via Tesseract | [OCR (Experimental)](ocr.md) |
| Threat model, download integrity, review | [Privacy and Security](privacy-security.md) |
| Workspace, stack table, IPC surface | [Architecture](architecture.md) |
| Toolchain, feature flags, test tiers | [Building from Source](building.md) |
