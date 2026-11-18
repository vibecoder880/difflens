---
title: "Phase 3: Diff + 3-pass alignment engine (similar, ort/e5-small, NW)"
status: todo
priority: P1
effort: 48h
---

# Phase 3: Diff + 3-Pass Alignment Engine

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.5 (ort, e5-small, tokenizers, mean pooling, 3-core latency), §2.8 (similar 3.2.1, strsim), §3 risk 1 (ort RC), risk 7 (3-core performance).

## Overview

**Priority:** P1 · **Status:** pending · **Effort:** ~48h
Build the heart of DiffLens in `difflens-core`: word/char inline diff via `similar` 3.2.1, and deterministic 3-pass paragraph alignment — (1) exact/structural anchors, (2) embedding cosine similarity via local multilingual-e5-small ONNX through ort 2.0.0-rc.15 with mean pooling + Needleman-Wunsch, (3) a move-repair window pass. Output is a typed `DiffResult` (ops, word segments, similarity, moved links) consumed by Phase 4 heuristics and Phase 7 UI. Embedding inference sits behind an `Embedder` trait with a deterministic fake implementation for tests, so the ort RC can be swapped cheaply.

## Key Insights

- `similar` 3.x (note major bump from 2.x) provides Myers/Patience + `iter_inline_changes` for replace ops — standardize on it; `diff-match-patch-rs` only if semantic cleanup proves needed (not planned).
- e5 protocol requires `"query: "` / `"passage: "` prefixes; mean pooling over last hidden state with attention mask then L2-normalize (~30 lines).
- ort is pinned `=2.0.0-rc.15` (RC ~2 years) — the trait boundary + fake embedder is the mitigation; candle/tract remain switchable.
- 3-core CPU budget: batch length-sorted with `intra_op_num_threads=3`; cap paragraphs per analysis (chunking with sliding window) so 1,000-paragraph docs stay seconds, not minutes.
- e5-small ONNX is bundled per the contract: `scripts/fetch-models.sh` downloads from pinned commit-hash URLs the exact files `onnx/model_quantized.onnx` (≈118 MB verified at planning; deliberately not `model_int8.onnx`/`model_uint8.onnx`, which are same-size look-alikes) and root `tokenizer.json` (presence re-verified at implementation — fetch fails loudly if missing) + SHA256 and byte size per file into `src-tauri/resources/models/e5-small/` (gitignored, fetched locally once and in CI).
- ORT linking strategy (decided): static linking via ort's default `download-binaries` feature — prebuilt static libraries are linked at build time, so no `onnxruntime` `.so`/`.dll`/`.dylib` needs bundling at runtime. If a target is ever forced onto `load-dynamic`, Phase 9 adds per-OS ORT shared-library bundling + release smoke coverage.

## Requirements

- [ ] `Embedder` trait + `FakeHashEmbedder` (deterministic, test-only) + `OrtE5Embedder` (ort + tokenizers 0.23.2).
- [ ] 3-pass alignment producing pair matches incl. moved detection; NW with O(n²) guard (chunk >2000 blocks).
- [ ] Word/char inline diff segments on Replace pairs; similarity scores via strsim.
- [ ] Change classification: Equal/Insert/Delete/Replace/Moved + formatting-only flag via style hints.
- [ ] Tauri command layer created here: `src-tauri/src/commands.rs` + `src-tauri/src/state.rs` with `run_diff`/`get_diff`/`cancel_diff`, `diff://progress` emission (extract/diff/align stages) and tokio cancellation; headless round-trip test green.
- [ ] Hand-labeled gold alignment set (≥20 pairs incl. moves/reorders) with measured precision/recall ≥ 0.85/0.80 on move detection; threshold + gap penalties recorded in the golden files.
- [ ] `cargo test -p difflens-core -- align diff` green on synthetic + fixture corpora (fake embedder, no network, no model needed).

## Architecture

`align/embeddings.rs`: `trait Embedder { fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> }`; `OrtE5Embedder` loads model + tokenizer.json from a resources dir, prefixes e5 query/passage, mean-pools, L2-normalizes, batches length-sorted. `align/anchors.rs` pass 1: unique exact normalized-text matches + heading/numbered anchors. `align/needleman.rs` pass 2: cosine similarity matrix → Needleman-Wunsch with block-kind-weighted gap penalties. `align/mod.rs` pass 3: for insert/delete runs, window-search ±K neighbors for cosine ≥ threshold → `Moved` links. `diff/mod.rs` assembles `DiffResult {pairs, changes}` where each `Change {op, a_block, b_block, word_segments, similarity, is_formatting_only}`; `diff/word.rs` runs `similar` word-level + char inline on Replace pairs.

## Related Code Files

- `core/src/align/mod.rs` (create — 3-pass orchestration + Alignment types)
- `core/src/align/embeddings.rs` (create — Embedder trait, FakeHashEmbedder, OrtE5Embedder)
- `core/src/align/anchors.rs` (create — pass 1)
- `core/src/align/needleman.rs` (create — pass 2, NW + cosine matrix + chunking)
- `core/src/diff/mod.rs` (create — DiffResult/Change assembly, classification)
- `core/src/diff/word.rs` (create — similar-based word/char segments)
- `scripts/fetch-models.sh` (create — pinned Xenova/multilingual-e5-small commit-hash URLs, exact files `onnx/model_quantized.onnx` + `tokenizer.json`, SHA256 + byte sizes → `src-tauri/resources/models/e5-small/`)
- `src-tauri/src/commands.rs` (create — `run_diff`/`get_diff`/`cancel_diff` orchestrating extract → diff → align, registered in `lib.rs`; later phases modify only)
- `src-tauri/src/state.rs` (create — `DiffState` holding job map + cancellation tokens in Tauri State; later phases extend it)
- `core/tests/align_tests.rs`, `core/tests/diff_tests.rs` (create)
- `core/tests/align_golden/*.json` (create — hand-labeled gold alignment set + recorded thresholds/penalties)
- `core/tests/fixtures/*.txt|docx` variant pairs (extend Phase 2 fixtures with A/B pairs)
- `.github/workflows/ci.yml` (modify — ubuntu-only feature-gated `embeddings-ort` job, see step 10)

## Implementation Steps

1. `embeddings.rs`: define `Embedder` trait + `FakeHashEmbedder` (seeded hash of tokens → 64-dim deterministic vectors, cosine behaves for exact/prefix matches); add `ort =2.0.0-rc.15` and `tokenizers 0.23.2` to workspace deps behind feature `embeddings-ort` (default on for app, off for fast test runs).
2. `OrtE5Embedder`: load `model.onnx` + `tokenizer.json` from a configured dir; session with `intra_op_num_threads(3)`; batch construction length-sorted with cap; e5 `"query: "` for doc A / `"passage: "` for doc B prefixes; mean pooling with attention mask; L2 normalize; return 384-dim vectors.
3. `scripts/fetch-models.sh`: download from pinned commit-hash URLs (`…/resolve/<commit>/onnx/model_quantized.onnx` and `…/resolve/<commit>/tokenizer.json` — name the exact variant `model_quantized.onnx`, never the same-size `model_int8.onnx`/`model_uint8.onnx`), verify SHA256 and recorded byte size (≈118 MB) per file, fail loudly if `tokenizer.json` is missing or any hash mismatches, place under `src-tauri/resources/models/e5-small/`; wire into release.yml and document local use.
4. `anchors.rs`: pass 1 — index normalized-text unique matches across docs; heading/numbered-item anchors (`BlockKind::Heading`, leading numbering); emit initial pair set marked `Anchor`.
5. `needleman.rs`: build cosine matrix over remaining blocks (chunk both docs in ≤2000-block windows with overlap so anchors anchor chunk boundaries); NW with match score = cosine, gap penalty tuned by block kind (headings cost more to skip); emit aligned pairs + gaps marked Insert/Delete.
6. `align/mod.rs` pass 3 move-repair: scan Insert/Delete runs; within ±K (default 12) blocks search for cosine ≥ 0.85 mutual-best matches → reclassify as `Moved` (drop from gap lists); cap O(n·K) work.
7. `diff/word.rs`: for Replace pairs run `similar::TextDiff` word granularity; char-level inline via `iter_inline_changes` for short pairs; similarity via `strsim::jaro_winkler` fallback when embeddings disabled (text-only mode for very large docs or `--no-model` runs).
8. `diff/mod.rs`: assemble `DiffResult`; classify `is_formatting_only` (identical normalized text but differing style hints); deterministic ordering by (a.ordinal, b.ordinal); serde-serializable types (Phase 7 mirrors them in TS).
9. Tests: synthetic corpora unit tests (pure insert, pure delete, move, reorder, split/merge) against `FakeHashEmbedder`; fixture-pair tests (two TXT variants, two DOCX variants → expected ops and moved links); NW table test; determinism test (two runs → identical serialized `DiffResult`); `#[ignore]` ort integration test gated by `DIFFLENS_E5_ONNX_PATH` + `DIFFLENS_E5_TOKENIZER` env vars, documented as optional manual check.
10. Gold alignment quality gate: hand-label ≥20 gold pairs (incl. moves/reorders across the TXT/DOCX/PDF fixtures) into `core/tests/align_golden/`; scoring harness computes precision/recall of pair matching + move detection; success gate precision ≥ 0.85 and recall ≥ 0.80 on moved links; record the move threshold (0.85) and NW gap-penalty values inside the golden files so future tuning is reviewable. bge-m3 note: these thresholds are calibrated for e5-small — re-running this gate with bge-m3 vectors is a precondition for shipping the Phase 5 bge-m3 opt-in entry.
11. Command layer (ownership of the app-facing diff entry point lands here): create `src-tauri/src/commands.rs` + `src-tauri/src/state.rs` implementing `run_diff(paths, options)` → extract → diff → align with `diff://progress` emission per stage (extract/diff/align; Phase 4 appends enrich), job ids for `get_diff`/`cancel_diff`, and a tokio cancellation token; register commands + state in `src-tauri/src/lib.rs`; headless test calls the command's inner function over two fixture files and asserts the round-trip result + progress events.
12. CI ort job: add to `ci.yml` an ubuntu-only job that runs `scripts/fetch-models.sh`, exports `DIFFLENS_E5_ONNX_PATH` + `DIFFLENS_E5_TOKENIZER`, then `cargo clippy --workspace --all-targets --features embeddings-ort -- -D warnings` and `cargo test -p difflens-core --features embeddings-ort -- --ignored`. This is the ubuntu-only ort gate (mac/windows ort paths are covered by the Phase 9 packaged-app release smoke).

## Todo

- [ ] Embedder trait + FakeHashEmbedder with tests
- [ ] OrtE5Embedder (ort pinned, pooling, batching) behind feature flag
- [ ] fetch-models.sh with pinned commit-hash URLs (`onnx/model_quantized.onnx` + `tokenizer.json`), SHA256 + byte sizes; model lands in resources dir
- [ ] Pass 1 anchors, pass 2 NW, pass 3 move-repair implemented
- [ ] Word/char diff segments + Change classification
- [ ] Gold alignment set (≥20 pairs) + precision/recall gate green
- [ ] `run_diff`/`get_diff`/`cancel_diff` commands + `diff://progress` + cancellation created; headless round-trip green
- [ ] CI `embeddings-ort` job added to ci.yml (ubuntu-only)
- [ ] Synthetic + fixture tests green; determinism test green
- [ ] `cargo test -p difflens-core -- align diff` green

## Success Criteria

- `cargo test -p difflens-core -- align diff` exits 0 with no network access and no model files present (fake embedder path).
- `cargo test -p difflens-core --features embeddings-ort -- --ignored` passes when the e5 model env vars point at fetched files; the ubuntu-only CI ort job (step 12) runs clippy + this ignored test with staged model artifacts on every push.
- Gold alignment set: measured precision ≥ 0.85 and recall ≥ 0.80 on move detection over `core/tests/align_golden/`; threshold + gap penalties recorded in the golden files.
- `run_diff` headless round-trip over two fixture files returns a `DiffResult` and emits `diff://progress` events for every stage; `cancel_diff` aborts a running job.
- Two runs over the same fixture pair produce byte-identical serialized `DiffResult`.
- A 1,000-block synthetic pair aligns in <60 s on the 3-core VM in release mode (`cargo build --release -p difflens-core` + timed test, informational).

## Risk Assessment

- ort 2.0.0-rc.15 breaks on a target OS → trait swap to candle/tract is a single `embeddings.rs` impl; CI compiles with `--features embeddings-ort` and runs the ignored ort test on ubuntu with staged model artifacts (feature-gated job, step 12), surfacing breakage early — mac/windows ort paths are exercised by the Phase 9 packaged-app release smoke.
- ort runtime library availability on end-user machines → resolved by static linking via ort's default `download-binaries` feature (no runtime `.so`/`.dll`/`.dylib` to bundle); if a target ever needs `load-dynamic`, Phase 9 bundles the per-OS ORT shared library and covers it in the release smoke.
- Community ONNX export changes → pinned commit-hash URLs + SHA256 + byte sizes in fetch-models.sh; re-export via optimum documented as backup.
- bge-m3 (568M) too slow on weak machines → strictly opt-in download-only (never bundled); Phase 5 ships it as an optional manifest entry only after this phase's gold alignment gate is re-run with bge-m3 vectors — the 0.85 cosine threshold is calibrated for e5-small and must be re-validated/tuned for bge-m3 before it ships.

## Security Considerations

- Model fetch verifies SHA256 before use; no inference ever reads outside the configured model dir.
- Documents never leave the process: embeddings are computed locally; the only host touched is 127.0.0.1 in Phase 5.

## Next Steps

Phase 4 enriches `Change` records with entity deltas, severity scores and audit entries; Phase 5 wires the download manager that fetches llama-server + GGUF + optional bge-m3.
