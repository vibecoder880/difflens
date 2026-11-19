# Building from Source

Standard Tauri 2 workspace: Rust workspace (`core/` + `src-tauri/`), Vite/React frontend at the root.

## Prerequisites

- [rustup](https://rustup.rs) stable — **MSRV 1.88** (workspace-enforced).
- Node.js 24 + npm.
- Linux: Tauri system libraries (same line CI uses):

```bash
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config
# optional, for OCR tests: tesseract-ocr tesseract-ocr-vie
```

## Build & run

```bash
git clone https://github.com/vibecoder880/difflens
cd difflens
npm ci                # frontend deps + @tauri-apps/cli
npm run tauri dev     # app with hot reload
npm run tauri build   # release installers
```

`npm run dev/test/build` run `scripts/export-labels.mjs` first, exporting the core's i18n label keys so frontend parity tests track the live backend list.

## Feature flags

- **`embeddings-ort`** (default off): real ONNX e5 embedder via `ort` (build-time downloads prebuilt ONNX Runtime). Without model files present it falls back — with a logged warning — to the deterministic fake hash embedder; the choice surfaces via `embeddings_used`.
- `scripts/fetch-models.sh` (`--test` / `--bundle`) and `scripts/fetch-pdfium.sh` stage pinned models/PDFium; CI sets `DIFFLENS_PDFIUM_STRICT=1` so PDF tests must run, not skip.

## Test suites

```bash
cargo fmt --check -p difflens-core
cargo clippy -p difflens-core --all-targets -- -D warnings
cargo test -p difflens-core
npm test              # vitest + EN/VI parity
npm run build
scripts/audit-egress.sh
```

| Tier | Content | Local |
|---|---|---|
| Core unit + integration | ~241 unit + ~53 integration tests incl. download/archive red-team suites | runs |
| Gold gates | alignment precision ≥ 0.85 / recall ≥ 0.80 | runs |
| PDF extraction | pinned PDFium | `#[ignore]`d without PDFium |
| Real embeddings | `embeddings-ort` + ONNX model | `#[ignore]`d |
| OCR e2e | tesseract + `vie` (`ocr-ci-ok` marker) | `#[ignore]`d |
| LLM roundtrip | pinned llama-server + Qwen3-0.6B | `#[ignore]`d |
| Frontend | vitest 52 cases + typecheck/build | runs |

## Disk notes

`target/` grows to 10–20 GB across configurations — build one platform locally, leave 3-OS release builds to CI, or relocate `CARGO_TARGET_DIR`. The `ort` feature adds a one-time ~100–300 MB build-time ONNX Runtime download.

## Conventions

[CONTRIBUTING.md](https://github.com/vibecoder880/difflens/blob/main/CONTRIBUTING.md): Conventional Commits · **egress rule** (new network code lives in `core/src/download/` and passes `scripts/audit-egress.sh`) · EN+VI strings in lockstep · LLM paths never alter diff results or severity.
