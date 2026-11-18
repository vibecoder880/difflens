# DiffLens

**Local-first, cross-platform document diffing with semantic alignment.**

DiffLens compares two documents — PDF, DOCX, TXT, XLSX, PPTX — entirely on
your machine. No accounts, no telemetry, no cloud: your files never leave
your device.

> 🚧 Active development — first production release is `v1.0.0` (tag-driven
> builds for Windows, macOS (Apple Silicon) and Linux via GitHub Actions).
> This README is finalized in the release phase with screenshots and usage
> docs.

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

## Status & docs

| Phase | Content | State |
|-------|---------|-------|
| 01 | Toolchain, workspace, CI/CD | ✅ |
| 02 | Extraction (PDF/DOCX/TXT/XLSX/PPTX) | 🚧 |
| 03 | Diff + 3-pass semantic alignment | 🚧 |
| 04 | Heuristic intelligence | 🚧 |
| 05 | llama-server sidecar + explanations | 🚧 |
| 06 | OCR (experimental) | 🚧 |
| 07 | Frontend (React/Tailwind/i18n) | 🚧 |
| 08 | Persistence + report export | 🚧 |
| 09 | Release engineering + docs | 🚧 |

## License

[MIT](./LICENSE) — see [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md)
(ships in the release phase) for bundled components and models.
