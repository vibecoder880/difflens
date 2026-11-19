# FAQ

**Is my data uploaded?**
No. Everything runs locally; the only network code is the Downloads manager (`core/src/download/`), used on your explicit click and checksum-gated. A CI job (`egress-audit`) enforces the invariant on every push. See [Privacy and Security](privacy-security.md).

**Why is the first alignment slow?**
One-time e5 model load, plus embedding every block of both documents. Later comparisons are faster. The LLM has the same pattern: first explanation includes sidecar spawn + GGUF load.

**Which languages?**
UI: English + Tiếng Việt (210 parity-tested keys). Engine: multilingual embeddings tuned for strong Vietnamese + English; heuristics parse VN entities natively. Other languages still word-diff, but without entity intelligence.

**Does OCR work on handwriting?**
No — printed text only, with honest quality caveats ([OCR](ocr.md)).

**Can I diff two folders?**
Not in v1: exactly two same-format documents per comparison.

**Where is history stored?**
`difflens.sqlite3` in the OS app-data directory (`%APPDATA%\app.difflens.desktop` / `~/Library/Application Support/app.difflens.desktop` / `~/.local/share/app.difflens.desktop`). Deleting an entry is permanent; nothing syncs.

**Why deterministic severity (no LLM)?**
Auditability: bands come from published rule points + a fixed lexicon, so reports are reproducible and every severity is explainable. The LLM can explain, never score — an architectural rule.

**Model licenses?**
e5-small: MIT · Qwen GGUFs: Apache-2.0 · llama-server: MIT · PDFium: BSD-3. Full inventory in [THIRD-PARTY-NOTICES.md](https://github.com/vibecoder880/difflens/blob/main/THIRD-PARTY-NOTICES.md); no AGPL/GPL.

**Intel Mac support?**
Not in v1 — Apple Silicon only. Windows x64 and Linux x64 are supported.

**Updater / auto-update plans?**
No updater is wired in v1; updates are manual via Releases. Signing infrastructure exists in CI, but the app never phones home.

**How to verify a release?**
`sha256sum -c checksums.txt` (Linux/macOS) or PowerShell `Get-FileHash` (Windows) — walkthrough in [Installation](installation.md).

**How to cite DiffLens?**

```bibtex
@software{difflens,
  author = {vibecoder880},
  title = {DiffLens: local-first document diffing with semantic alignment},
  url = {https://github.com/vibecoder880/difflens},
  version = {1.0.1},
  year = {2026}
}
```
