# Validation Report — DiffLens v1.0.0 Plan

- **Date:** 2026-11-18
- **Validator:** final validation gate before `/ak-cook` (per `ak-plan` validate-workflow + validate-question-framework + verification-roles)
- **Scope:** `plan.md`, `phase-01..09`, `research/researcher-01-stack-report.md`, `reports/red-team-review.md` incl. Resolution section
- **Inputs:** critical-question framework answers with plan evidence, whole-plan sweep (7 checks), filesystem reality check on the build VM, `ak plan validate` exit code.

## Verdict

**GO** for `/ak-cook`. Residual issues: **0 BLOCKER · 9 NOTE** (all carry-into-implementation items; no plan-file changes required).

---

## 1. Critical Questions Validation

Framework categories per `validate-question-framework.md` (Architecture / Assumptions / Tradeoffs / Risks / Scope). Status: **ANSWERED** = answerable from plan files with evidence; **FENCED** = answerable but the plan explicitly defers final confirmation to implementation time with a loud-failure gate; **FLAGGED** = cannot be fully answered from the plan alone.

### Architecture

| # | Question | Answer + Evidence | Status |
|---|----------|-------------------|--------|
| A1 | How is the diff pipeline exposed to the UI, and which phase owns it? | `src-tauri/src/commands.rs` + `state.rs` with `run_diff`/`get_diff`/`cancel_diff`, `diff://progress` (extract/diff/align → +enrich in Phase 4), tokio cancellation — **created in Phase 3** (phase-03:35, step 11 at phase-03:71), modified by 4/5/6/7/8. Red-team blocker F1 resolved as directed. | ANSWERED |
| A2 | Static or dynamic ONNX Runtime linking for release artifacts? | Static via ort default `download-binaries` (phase-03 Key Insights, phase-03:99); no runtime `.so/.dll/.dylib` to bundle; `load-dynamic` fallback ⇒ Phase 9 per-OS bundling + release smoke (phase-09:38). Red-team A3 resolved. | ANSWERED |
| A3 | Per-OS bundling strategy for PDFium/model files? | Target-specific `tauri.linux/macos/windows.conf.json` override `bundle.resources` globs, decided in Phase 1 (phase-01:37, :44). Red-team A5 second half resolved. | ANSWERED |
| A4 | Workspace/crate layout? | Root `Cargo.toml` (members `src-tauri`, `core`, resolver 2, shared `[workspace.dependencies]`), `difflens` app shell + `difflens-core` headless lib (plan.md:36, phase-01:37, :61). | ANSWERED |
| A5 | Frontend state/routing? | TanStack Router (code routes, SPA) + Query + Virtual; no Zustand — local state + query cache (phase-07:40). | ANSWERED |

### Assumptions

| # | Question | Answer + Evidence | Status |
|---|----------|-------------------|--------|
| B1 | Is `$HOME/.cache` on a different/larger mount for `CARGO_TARGET_DIR` relocation? | Plan does NOT assume it — step 2 measures per mount first and selects the measured-largest mount, with a <2 GB-headroom abort criterion that drops `embeddings-ort` locally and rides the Phase 3 CI ort job (phase-01:58, :90). **Reality check:** this VM has a single ext4 root fs (10G avail) — relocation is a no-op here and the abort criterion is live. | ANSWERED (procedure) — see NOTE N2 |
| B2 | e5-small exact filenames + `tokenizer.json` presence? | Plan names exact variant `onnx/model_quantized.onnx` (not the same-size `model_int8`/`model_uint8` look-alikes), byte size + SHA256 recorded, loud failure if `tokenizer.json` missing (phase-03:26, step 3 phase-03:63). Presence itself is re-verified at implementation. | FENCED (loud-failure gate) |
| B3 | tauri-action tag? | `@v0` rolling tag (v0.6.x line), resolve current major tag / prefer commit-SHA pin at implementation; Risk Assessment records the UNVERIFIED→resolved history (phase-01:23, :37, :64, :88). Red-team A1 resolved. | FENCED |
| B4 | GitHub runner labels? | All workflows pin `ubuntu-24.04`; matrix `ubuntu-24.04`/`macos-14`/`windows-latest`; re-verify labels (incl. `macos-14` vintage) at each release; ubuntu-22.04 deprecation documented (phase-01:37, :64, :89). Red-team A2/C3 resolved. | ANSWERED |
| B5 | llama-server `--port 0` ephemeral-port mechanism? | Startup-log bind-line parsing is primary with a canned-log unit test; `--port 0` support verified at implementation and recorded in manifest notes (phase-05:55). Red-team A6 resolved. | FENCED |
| B6 | Qwen2.5-3B-Instruct Q4_K_M low-RAM fallback manifest entry? | Included in manifest; exact filename/size/checksum explicitly marked "verified at implementation — UNVERIFIED at planning" (phase-05:38, :54, :76, :84). Red-team F6 resolved. | FENCED |
| B7 | Does the VM have a display for manual E2E (Phases 7/9)? | Plan records the display requirement (GUI session or `xvfb-run` recipe) in `docs/dev-setup.md` from Phase 1 step 1; Phase 7 manual E2E cross-references it (phase-01:57, phase-07:84). Cannot be confirmed from plan or this shell session. | FLAGGED — NOTE N9 |
| B8 | pnpm availability on VM? | Not installed yet; Phase 1 step 1 installs via `npm i -g pnpm` (phase-01:57). Reality: node v24.22.1 + npm 11.19.1 present, so the install path works. | ANSWERED |

### Tradeoffs

| # | Question | Answer + Evidence | Status |
|---|----------|-------------------|--------|
| C1 | DOCX: own parser vs docx-rs? | Raw zip+roxmltree own parser (~500–800 lines) for run/order/tracked-changes fidelity; graceful degradation on exotic OOXML (plan.md:28, phase-02:22, research §2.3). | ANSWERED |
| C2 | `similar` 3.x vs diff-match-patch-rs? | Standardize on `similar` 3.2.1 (Myers/Patience + `iter_inline_changes`); dmp only if semantic cleanup proves needed — not planned (phase-03:22). | ANSWERED |
| C3 | OCR: bundled ONNX vs detected Tesseract CLI? | Tesseract CLI only, honest experimental label; ONNX OCR is a research note only (no product code, no cargo feature), default NO-GO, v2 planning (plan.md:30, phase-06:18, :37, :58). Red-team S3 resolved. | ANSWERED |
| C4 | llama-server bundled vs downloaded? | Downloaded sidecar, never bundled; pinned b-tag + SHA256 manifest, fails-first (plan.md:31, phase-05:22). | ANSWERED |
| C5 | Ship bge-m3 second embedding model? | Strictly opt-in download-only, never bundled; manifest entry gated on re-running the Phase 3 gold alignment gate with bge-m3 vectors (0.85 threshold is e5-calibrated) (plan.md:36, phase-03:70, :101, phase-05:38, phase-07:61). Red-team S2 resolved. | ANSWERED |
| C6 | shadcn Radix vs Base UI default drift? | Pin Radix explicitly via `shadcn init` radix flag (phase-01:59, research §2.9). | ANSWERED |
| C7 | Auto-updater wiring in v1? | Post-1.0 scope; keypair + GitHub secrets generated day 1, no `plugins.updater`/`createUpdaterArtifacts`/endpoint in v1 (phase-01:65, phase-09:22). Red-team S4 resolved. | ANSWERED |

### Risks

| # | Question | Answer + Evidence | Status |
|---|----------|-------------------|--------|
| D1 | ort RC breakage on a target? | `Embedder` trait + FakeHashEmbedder swap boundary; ubuntu-only CI job compiles `--features embeddings-ort` and runs the ignored ort test with staged model artifacts; mac/win covered by Phase 9 packaged-app smoke (phase-03:98, step 12 phase-03:72). Red-team F5 resolved. | ANSWERED |
| D2 | VM disk exhaustion mid-plan? | Measured itemized budget, relocation target by measurement, clean cadence, <2 GB abort → drop `embeddings-ort` locally (phase-01:58, :90). | ANSWERED — see NOTE N2 |
| D3 | llama.cpp release drift? | Committed manifest pins exact b-tag + per-OS assets + SHA256 + provenance; fallback URLs carry own checksums; fails-first (phase-05:38, :54). | ANSWERED (exact tag picked at implementation — FENCED) |
| D4 | PDFium C-layer crash on hostile PDFs? | Success criterion reworded to "no Rust-level panics"; SIGSEGV/SIGABRT documented as known limitation; CVE surface re-checked each release (phase-02:80, :86, phase-09:86). Red-team SEC5 resolved. | ANSWERED |
| D5 | CI stays green across phases? | Phase 2 adds PDFium staging + DIFFLENS_PDFIUM_PATH + "pdf tests ran (not skipped)" assert on Linux (phase-02:64); Phase 3 adds ubuntu ort job (phase-03:72); Phase 5 wires egress audit (phase-05:60). Red-team F3/F5 resolved. | ANSWERED — see NOTE N4 (file-list omissions) |

### Scope

| # | Question | Answer + Evidence | Status |
|---|----------|-------------------|--------|
| E1 | Is Phase 8 (persistence/reports) in scope? | Yes — acceptance criterion **(h)** added to plan.md Requirements and mapped to Phase 8; Phase 9 closes it (plan.md:61, phase-09:18). Red-team S1 resolved. | ANSWERED |
| E2 | Module enumeration includes Phase 8 crates? | plan.md Architecture + phase-01 step 5 enumerate `store`/`report` (stubs added in Phase 8) (plan.md:36, phase-01:61). Red-team C1 resolved. | ANSWERED |
| E3 | Phase 7 ships wrappers for Phase 8 commands? | No — `listSessions`/`deleteSession`/`exportReport` explicitly deferred; Phase 8 adds them (phase-07:58, phase-08:49, :60). Red-team F9 resolved. | ANSWERED |

**Framework conclusion:** every critical question is answerable from the plan set; 5 are explicitly fenced for implementation-time verification with loud-failure gates (B2, B3, B5, B6, and the llama.cpp exact b-tag in D3). Only B7 (display availability) cannot be answered from the plan — it is procedurally handled (NOTE N9).

---

## 2. Whole-Plan Sweep

### (a) YAML frontmatter — PASS

- `plan.md`: full required schema present — title, description, status: pending, priority: P1, effort: 272h, issue, branch, tags, blockedBy, blocks, created: 2026-11-17 (output-standards.md schema).
- All 9 phase files carry valid YAML frontmatter (title, status: todo, priority, effort). Output-standards mandates frontmatter for plan.md; phase frontmatter is valid YAML and parses (confirmed by `ak plan validate` exit 0).
- Effort sum check: 24+40+48+24+32+16+48+24+16 = **272h = plan.md `effort`** ✓.

### (b) plan.md links — PASS with NOTE N1

- Phases table links `./phase-01..09` all resolve to real files; `./research/researcher-01-stack-report.md` resolves ✓.
- Link text: the table uses filenames (`[phase-01-bootstrap-ci.md](...)`) — plan-organization.md's template comment says "Link text MUST be human-readable names (not filenames)". Links resolve and are clickable, so this is cosmetic → **NOTE N1**.

### (c) File-ownership matrix — PASS (no collisions)

Creation ownership per phase (create vs modify), all conflicts free:

- Phase 1: workspace root `Cargo.toml`, `src-tauri/*` (conf, capabilities, main/lib/build.rs), `core/Cargo.toml`+`lib.rs`, frontend scaffolds (`package.json`, `vite.config.ts`, `index.html`, `src/main.tsx`, `App.tsx`, `globals.css`), `.gitignore`, `LICENSE`, `README.md` (stub), `docs/dev-setup.md`, `scripts/setup-linux-deps.sh`, `scripts/disk-hygiene.sh`, `.github/workflows/{ci,release}.yml`.
- Phase 2: `core/src/model.rs`, `extract/*` (incl. `pdfium_loader.rs`, `normalize.rs`), `core/tests/extract_tests.rs` + fixtures, `scripts/fixtures/generate.mjs`, templates, `scripts/fetch-pdfium.sh`; ci.yml **modify** ✓.
- Phase 3: `core/src/{align,diff}/*`, `scripts/fetch-models.sh`, `src-tauri/src/{commands,state}.rs` (create — F1 fix), align/diff tests + golden; fixtures **extend** Phase 2 (explicitly annotated); ci.yml **modify** ✓.
- Phase 4: `core/src/heuristic/*`, heuristic tests + golden; commands/state **modify** ✓.
- Phase 5: `core/src/download/*` (mod/checksum/manifest.json), `core/src/llm/*`, download/llm tests, `scripts/audit-egress.sh`; commands/state/capabilities **modify** ✓.
- Phase 6: `core/src/ocr/*`, ocr tests, `reports/ocr-onnx-spike-research.md`; model.rs / extract/mod.rs / commands.rs / generate.mjs **modify** ✓.
- Phase 7: routes, compare/downloads/ui components, `src/lib/*`, `src/i18n/*`, `__tests__/*`; main/App/globals/commands **modify** ✓; session wrappers deferred ✓.
- Phase 8: `core/src/store/*`, `core/src/report/*`, store/report tests; commands/state/tauri.ts/types.ts/history.tsx **modify** ✓; `SessionCard.tsx` create (owned by no earlier phase) ✓.
- Phase 9: icons + `assets/icon.png`, `core/tests/e2e_test.rs`, `THIRD-PARTY-NOTICES.md`, `scripts/{gen-notices,release-checklist}`, `LicensesScreen.tsx`, `docs/{build,privacy}.md` (new), conf/Cargo/README/workflows **modify** ✓.
- Sequential same-file modification chains (commands.rs: 3→4→5→6→7→8; state.rs: 3→5→8; ci.yml: 1→2→3→5; release.yml: 1→3→9; generate.mjs: 2→6; tauri.ts/types.ts: 7→8; model.rs: 2→6) are ordered, never parallel-owned ✓.

Two list-accuracy gaps (not collisions) → **NOTE N4** and **NOTE N5**.

### (d) Commands in success criteria — PASS

All verified as real, valid invocations:

- `cargo build -p difflens`, `cargo test --workspace`, `cargo clippy [-p|--workspace] ... -- -D warnings` — valid cargo grammar (app crate name `difflens` consistent per red-team sweep; see NOTE N7 for the implicit naming assumption).
- Multi-filter `cargo test -p difflens-core -- align diff` / `-- download llm` / `-- store report` — libtest multiple filters **stabilized in Rust 1.83.0** (2024-11-28, `cargo test -- foo bar`); plan installs current stable via rustup ⇒ valid. Red-team F2 fix verified in phase-03:37/:77/:89, phase-05:34/:70/:74, phase-08:34/:72/:76.
- Single-filter forms `cargo test -p difflens-core extract|heuristic|ocr` — valid (one positional TESTNAME).
- `cargo test -p difflens-core --features embeddings-ort -- --ignored` — valid.
- `cargo clippy --workspace --all-targets --features embeddings-ort -- -D warnings` — valid: `--features` with an explicit package selection (`--workspace`) is allowed; the "not allowed in the root of a virtual workspace" error fires only when `--features` is used with no package selection. Root workspace uses resolver 2 ✓.
- `pnpm build` / `pnpm test` / `pnpm exec tsc --noEmit` / `pnpm tauri init|signer generate|icon` — valid (pnpm installed in Phase 1).
- `gh repo create vibecoder880/difflens --public --source . --push`, `gh run list --limit 1`, `gh release view v1.0.0` — valid gh invocations; token scopes (`repo`, `workflow`) suffice (verified below).
- `git grep -l "PRIVATE KEY" $(git rev-list --all)`, `git grep -iE "telemetry|analytics" src core src-tauri`, `scripts/audit-egress.sh`, `df -h`, `cargo about generate` (in `scripts/gen-notices.sh`), `pkg-config --modversion webkit2gtk-4.1` — all valid as used.

### (e) Acceptance (a)–(h) → phase success criteria — PASS

| AC | Delivering phase success criteria |
|----|-----------------------------------|
| (a) cargo test green incl. fixtures | phase-02:77 (fixtures included), phase-03:89, phase-04:72, phase-06:73, phase-08:78 (`cargo test --workspace`), phase-09:76 (incl. e2e_test.rs) |
| (b) local builds green | phase-01:80 (`cargo build -p difflens` + `pnpm build`) |
| (c) 3-OS artifacts + draft release on tag push | phase-01:82 (v0.1.0-alpha.0), phase-09:77 (`gh release view v1.0.0`) |
| (d) E2E DOCX/PDF diff + severity + audit + optional LLM | phase-02:79, phase-03:92–93 (run_diff round-trip), phase-04:75 (enrich stage), phase-05:76 (LLM manual gate), phase-07:84–85 (UI E2E) |
| (e) EN+VI i18n switchable | phase-07:84 (EN↔VI toggle) + parity test phase-07:86/step 3 |
| (f) README + screenshots + license + attribution | phase-09:79 |
| (g) no egress except download manager | phase-05:75 (audit-egress.sh), phase-09:78 (re-run) |
| (h) history + HTML/MD export with audit trail | phase-08:79 (history, HTML opens from disk, Markdown renders) |

Every criterion maps to ≥1 phase success criterion ✓ (and plan.md:54–61 states the same mapping).

### (f) Forward references — PASS

- Phase 1 release.yml calls `scripts/fetch-pdfium.sh` / `fetch-models.sh` (created Phases 2–3) via guarded hooks `if [ -f scripts/fetch-pdfium.sh ]; then ... fi` (phase-01:64) ✓ — red-team F4 fix verified.
- Phase 5/6/7 reference `commands.rs` with explicit "file created in Phase 3" annotations ✓.
- Phase 7 settings clear-history explicitly "stub behind feature flag until Phase 8 commands land" ✓.
- Phase 6 `Format::PdfOcr`/`StyleHints.ocr` declared as model.rs modify + Phase 7 TS-mirror note (red-team C2 fix) ✓.
- Phase 1 ci.yml description foreshadows Phase 2 PDFium staging + Phase 3 ort job ✓.
- No phase consumes a file/script before its owning phase (beyond the guarded hooks).

### (g) Contradiction scan — PASS

- **Versions:** tauri 2.12 (plan.md:36, phase-01:37, :60) ✓; ort `=2.0.0-rc.15` (plan.md:29, phase-01:61, phase-03:24) ✓; similar 3.2/3.2.1 ✓; tokenizers 0.23/0.23.2 (caret-compatible, not contradictory) ✓; zip 9.7, roxmltree 0.21, calamine 0.38, pdfium-render 0.9.5, i18next 27, React 19/Vite 7/Tailwind 4 — all consistent across plan.md, phase-01, phase-03, phase-07, research.
- **Paths:** `src-tauri/resources/pdfium/`, `src-tauri/resources/models/e5-small/` (gitignore covers both, phase-01:58), `core/tests/fixtures/`, `core/tests/golden/` (P4) vs `core/tests/align_golden/` (P3) — distinct, no clash; `app_data_dir/bin`, `app_data_dir/difflens.db` consistent.
- **Feature flags:** `embeddings-ort` (defined P3, referenced P1/P9) ✓; OCR `ocr_enabled` default off + experimental label consistent P6↔P7 ✓.
- **Runner labels:** `ubuntu-24.04` in phase-01 Architecture, step 8, Risk Assessment, phase-03 ort job ("ubuntu-only"), research §2.9 ✓; `macos-14` vintage re-verification noted ✓.
- **Egress exclusions:** plan.md:67 (`core/src/download` + `core/src/llm/client.rs`) == phase-05:60 grep exclusions ✓; Phase 7 "no direct fetch in src/" covered by same grep (`--include *.ts/tsx`) ✓.
- **Updater:** phase-01 "Do NOT wire plugins.updater" ↔ phase-09 "no updater artifacts in v1" ↔ plan.md Security ✓.
- **Red-team Resolution section:** spot-checked all 30 claims (A1–A7, F1–F10, S1–S4, SEC1–SEC6, C1–C3) against current plan text — every fix is present in the files as described. Verified by direct read of phase-01:23/:37/:57–58/:64–65/:88–92, phase-02:64/:80, phase-03:26–27/:63/:70–72/:98–101, phase-05:38/:54–55/:60/:76/:84–93, phase-06:37/:44/:58, phase-07:53/:58/:65, phase-08:57/:61/:77, phase-09:22/:38/:59/:61/:77–78, plan.md:36/:54–61/:67.
- Red-team "Verification Results" (0 Failed) consistent with plan state; no `[UNVERIFIED]` tags remain in plan/phase files except explicitly-fenced implementation-time checks (5 items, all with loud-failure gates).

---

## 3. Filesystem Reality Check (build VM)

| Check | Result |
|-------|--------|
| `df -h /home/khanh/Documents` | `/dev/mapper/pve-vm--109--disk--0` ext4, 48G total, 35G used, **10G avail** (78%) — single root filesystem; no second mount exists (/, /home, /tmp identical) |
| Repo files beyond plans dir | `/home/khanh/Documents/difflens/` contains **only `plans/`** ✓ (unrelated sibling projects in ~/Documents are not repo files) |
| `node --version` | **v24.22.1** ✓ (npm 11.19.1 present) |
| pnpm | not installed — Phase 1 installs it (`npm i -g pnpm`) ✓ consistent |
| rustup / cargo | not installed ✓ expected — Phase 1 installs via `rustup-init -y --profile minimal`; curl/wget/git present, so the step would work |
| `gh auth status` | exit 0 — logged in as **vibecoder880**, scopes `gist, read:org, repo, workflow` ✓ sufficient for `gh repo create --push` + Actions |
| OS / resources | Ubuntu **24.04.5 LTS** ✓ (matches plan's pinned distro; `libwebkit2gtk-4.1-dev` available in Noble repos), 3 cores ✓, 6 GB RAM ✓ (matches plan assumptions) |
| webkit2gtk-4.1 / pkg-config | not installed ✓ expected — Phase 1 `scripts/setup-linux-deps.sh` installs them; assert step will validate |
| `zip` CLI | **not installed** — Phase 2 fixture generation uses the `zip` CLI and no phase installs it → **NOTE N3** |
| tesseract | not installed ✓ expected — Phase 6 detection skips gracefully; CI apt-installs it |
| Phase 1 toolchain steps viable? | Yes: rustup-init (curl present, egress works — gh/curl functional), apt dev packages available on 24.04, `npm i -g pnpm` viable, `gh repo create` authorized. Disk is the only real constraint (N2). |

---

## 4. `ak plan validate`

```
$ ak plan validate /home/khanh/Documents/difflens/plans/20261117-difflens
[OK] ./plans/20261117-difflens is a valid plan directory
EXIT CODE: 0
```

---

## 5. Residual Issues

### BLOCKER

None.

### NOTE (acceptable — carry into implementation)

1. **N1 — plan.md Phases-table link text is filenames.** Links resolve, but plan-organization.md's template guidance wants human-readable names ("Good: [Setup Environment](./phase-01-setup.md)"). Cosmetic; fix opportunistically (one line per row). Does not gate cooking.
2. **N2 — No second mount on the VM; disk budget is tight.** Single ext4 root fs with 10G avail. The plan's itemized budget (rustup ~1G + registry 1–1.5G + node_modules 0.5–1G + debug target 4–8G + ORT 0.3G + resources 0.15G ≈ 6.8–11.8G) may exhaust 10G. The plan's measurement-first step 2 and <2 GB-headroom abort criterion (drop `embeddings-ort` locally, rely on the Phase 3 CI ort job) are the designed response — expect that contingency to actually fire on this VM. Do not skip the step-2 `df`/`du` measurement.
3. **N3 — `zip` CLI not installed on the VM.** Phase 2 step 9 builds OOXML fixtures with the `zip` CLI; no phase installs it (`scripts/setup-linux-deps.sh` omits it). One-line fix at Phase 2 time (`apt install zip`) or add to the setup script while implementing Phase 1.
4. **N4 — Two Related-Code-Files omissions.** Phase 3 step 3 wires `fetch-models.sh` into `release.yml` but `release.yml` is absent from Phase 3's Related Code Files; Phase 5 step 8 wires `audit-egress.sh` into `ci.yml` but `ci.yml` is absent from Phase 5's list. Ownership remains sequential and conflict-free; implementers must remember both files are modified in those phases.
5. **N5 — `THIRD-PARTY-NOTICES.md` placeholder ambiguity.** Phase 1 step 7 "notes a placeholder for Phase 9"; Phase 9 lists it "(create)". Intent is clear (Phase 9 owns the completed file); Phase 1 should only mention it in README, not create the file — carry this reading into implementation.
6. **N6 — Five implementation-time verification fences.** tauri-action exact tag/commit-SHA; llama.cpp exact b-tag; e5-small `tokenizer.json` presence; Qwen2.5-3B-Instruct Q4_K_M exact filename/size/checksum; llama-server `--port 0` support. All fenced with loud-failure gates in the plan — resolve each at the phase where the plan says to.
7. **N7 — `cargo build -p difflens` assumes the src-tauri package is named `difflens`.** Consistent with red-team's sweep and tauri-init naming from the `difflens` project name, but the package name is never stated explicitly in phase-01. If scaffolding names it differently, adjust the `-p` argument (or rename the crate) in Phase 1.
8. **N8 — VM memory pressure.** 6 GB RAM with only ~1.2 GB currently available and swap nearly full at validation time. Plan's mitigations (mock SSE server for dev tests, Qwen2.5-3B low-RAM manual gate) cover the risky paths; debug cargo builds are fine but avoid running heavy processes concurrently during builds.
9. **N9 — Display availability for manual E2E unconfirmed.** Plan correctly records the GUI-or-`xvfb-run` recipe in `docs/dev-setup.md` (Phase 1) and Phase 7/9 reference it. Whether a GUI session exists on this VM could not be verified from the shell — the xvfb path is the safe default for Phase 7/9 manual gates.

---

## 6. Final Verdict

**GO** for `/ak-cook /home/khanh/Documents/difflens/plans/20261117-difflens/plan.md`.

Rationale: `ak plan validate` exits 0; all 7 sweep checks pass (frontmatter valid, links resolve, zero file-ownership collisions, every success-criteria command is a valid invocation — including the Rust 1.83 multi-filter syntax and the virtual-workspace `--features`+`--workspace` combination, acceptance (a)–(h) each map to ≥1 phase success criterion, forward references guarded, no cross-phase contradictions); all 30 red-team findings verified as applied in the plan text; filesystem reality check confirms node v24, gh auth as `vibecoder880` with sufficient scopes, Ubuntu 24.04.5 with 3 cores/6 GB matching plan assumptions, and no pre-existing repo files. The 9 residual NOTEs are carry-into-implementation items (disk contingency N2 being the most consequential) — none requires a plan change before cooking.
