# Red Team Review — DiffLens v1.0.0 Implementation Plan

- **Date:** 2026-11-17
- **Protocol:** 4-persona adversarial review per `ak-plan` red-team workflow (Assumptions Destroyer, Failure Mode Analyst, Scope & Complexity Critic, Security Adversary). Severity scale mapped to BLOCKER / MAJOR / MINOR.
- **Files reviewed:** `plan.md`, `phase-01-bootstrap-ci.md` … `phase-09-release.md`, `research/researcher-01-stack-report.md`.
- **Evidence filter:** every finding carries `file:line` citations from the plan set. External claims were checked against the research report's own VERIFIED/UNVERIFIED flags and two live checks (GitHub runner-images deprecation notice, tauri-action releases, HF `Xenova/multilingual-e5-small` ONNX tree).
- **Note:** no plan files were modified. All fixes are instructions for the plan authors.

**Severity counts: 3 BLOCKER · 12 MAJOR · 13 MINOR.**

---

## Persona 1 — Assumptions Destroyer

### A1. `tauri-apps/tauri-action@v2` is a tag that does not exist — BLOCKER
- **Location:** phase-01-bootstrap-ci.md:63 ("`tauri-apps/tauri-action@v2`"), also implied at phase-01:37 and phase-09:38/49 (release.yml modifications inherit the broken pin).
- **Flaw:** The plan hard-pins `@v2`. The research report (researcher-01-stack-report.md:15) explicitly marks the tauri-action tag "UNVERIFIED" and says only "2.x line", yet the plan silently converted that UNVERIFIED flag into a hard pin. tauri-action's release/tag line is `v0` (latest v0.6.x); `@v2` resolves to nothing.
- **Failure scenario:** Step 10 of Phase 1 pushes tag `v0.1.0-alpha.0`; `release.yml` fails instantly with "unable to resolve action tauri-apps/tauri-action@v2". Phase 1's success criterion "tag yields a draft release with installers for all 3 OSes" (phase-01:81) can never pass, and acceptance (c) is unreachable. Every later release-gated criterion (phase-09:77) fails on the same pin.
- **Evidence:** phase-01:63 vs researcher-01-stack-report.md:15 ("exact tag UNVERIFIED"); tauri-action GitHub releases show `v0` rolling tag (v0.6.x line).
- **Fix:** phase-01-bootstrap-ci.md, Implementation Steps step 8 and Architecture section: replace `@v2` with `@v0` and add an explicit verification step ("resolve the current major tag at implementation time; prefer pinning by commit SHA"). Update the Risk Assessment at phase-01:88 to record that the research flagged this UNVERIFIED and it was resolved by check.

### A2. CI pinned to `ubuntu-22.04`, which entered deprecation 2026-09-17 — BLOCKER
- **Location:** phase-01-bootstrap-ci.md:37 (Architecture: "`ci.yml` … on ubuntu-22.04", "matrix `ubuntu-22.04` / `macos-14` / `windows-latest`"), phase-01:63, phase-01:86.
- **Flaw:** The plan freezes the Linux runner at ubuntu-22.04 while executing Nov 2026 → spring 2027. GitHub's runner-images deprecation notice began 2026-09-17 with full removal 2027-04-17. The research report itself contradicts the plan: researcher-01-stack-report.md:39 says "This is what CI (ubuntu-24.04 runner) and the dev VM need."
- **Failure scenario:** CI is green during Phase 1, then brownout/forced-migration windows start failing `ci.yml` and `release.yml` mid-plan (phases 2–9 span ~272 h, well into the deprecation window). Acceptance (c) fails intermittently or permanently if the plan slips past April 2027. Also note `macos-14` is the same vintage and should be re-checked at implementation time.
- **Evidence:** phase-01:37/63/86 vs researcher-01-stack-report.md:39; actions/runner-images issue #14254 (deprecation 2026-09-17, unsupported 2027-04-17).
- **Fix:** phase-01-bootstrap-ci.md, Architecture and Implementation Steps step 8: change both workflows to `ubuntu-24.04` (matches the research), and add a note to re-verify runner labels at each release. Keep the `pkg-config --modversion webkit2gtk-4.1` assert (phase-01:56) — 4.1 is present on 24.04.

### A3. ONNX Runtime runtime-library is never bundled or linked for release artifacts — MAJOR
- **Location:** phase-03-diff-alignment.md:38, :54 (OrtE5Embedder, `embeddings-ort` default-on for app); phase-09-release.md:38/52–59 (release pipeline and bundling list never mention ORT); researcher-01-stack-report.md:73.
- **Flaw:** The research states ORT's runtime `.so/.dll/.dylib` (~10–25 MB) "must be bundled or pulled via the feature's static linking option". No phase does either: `bundle.resources` (phase-01:37) lists only `resources/*` (pdfium, models), and phase-09's bundle-target list (msi/nsis, dmg, deb/appimage) has no ORT step. `ort`'s default `download-binaries` feature places the lib in the build tree, not the bundle.
- **Failure scenario:** CI builds pass (ORT present in the build environment), tauri-driver smoke is optional and CI-side (phase-09:57), so v1.0.0 installers ship with an app that crashes/fails to init embeddings on end-user machines — acceptance (d)'s embedding-backed alignment silently degrades to `strsim` fallback (phase-03:60) or panics at model load.
- **Evidence:** researcher-01-stack-report.md:73; absence in phase-01:37, phase-03, phase-09:38–59.
- **Fix:** phase-03-diff-alignment.md, Architecture or Risk Assessment: decide static vs dynamic ORT linking. If dynamic, add to phase-09-release.md Architecture + step 2 a per-OS bundling step (resource or `externalBin` naming per target triple) and a release smoke that loads the model in the packaged app.

### A4. VM disk budget is asserted, never itemized; `CARGO_TARGET_DIR` relocation target is unverified — MAJOR
- **Location:** phase-01-bootstrap-ci.md:24 (10–20 GB target claim), :56–57 (rustup + `CARGO_TARGET_DIR=$HOME/.cache/difflens-target`), :65/75 (≥5 GB free criterion), researcher-01-stack-report.md:43.
- **Flaw:** The plan assumes `$HOME/.cache` is on a mount other than the 9.5 GB-free filesystem ("ext4 cache"). Nothing in the plan verifies this. Itemized budget on the 9.5 GB disk: rustup minimal stable ≈ 1 GB; cargo registry ≈ 1–1.5 GB; `node_modules` + pnpm store ≈ 0.5–1 GB; a Tauri debug `target/` ≈ 4–8 GB (research:43 says 5–8 GB tight); ORT build download ≈ 0.3 GB; pdfium + e5-small resources ≈ 0.15 GB. Sum plausibly exceeds 9.5 GB, and Phase 3's success criterion (phase-03:79) additionally demands a **release** build on the VM.
- **Failure scenario:** `cargo build` dies mid-plan with ENOSPC after Phase 3 adds ort/tokenizers; the ≥5 GB-free criterion (phase-01:75) fails and there is no contingency. If `$HOME/.cache` is the same 9.5 GB mount, the "mitigation" is a no-op.
- **Evidence:** phase-01:24, :56–57, :65, :75; researcher-01-stack-report.md:43/62; no step anywhere measures `df -h` per mount before choosing the relocation target.
- **Fix:** phase-01-bootstrap-ci.md, Implementation Steps step 2: add a measured disk-budget table (each component's actual footprint via `du`/`df`), select the relocation mount by measurement, and add an explicit abort/escalation criterion (e.g., if <2 GB headroom remains after the debug build, drop `embeddings-ort` from local dev builds and rely on CI for ort-featured verification — which also requires fix F5).

### A5. e5-small ONNX export: filenames, tokenizer presence, and 118 MB bundle impact unverified — MAJOR
- **Location:** phase-03-diff-alignment.md:26 ("model.onnx quantized variant + tokenizer.json"), :54–56 (fetch-models.sh, SHA256s "recorded in the script"), :68; researcher-01-stack-report.md:78/:162 (filenames beyond standard layout UNVERIFIED; tokenizer.json presence assumed).
- **Flaw:** The plan commits SHA256s in a script for files whose exact names/paths the research flagged UNVERIFIED, and never states the size. Live check: `onnx/model_quantized.onnx` = 118 MB exists (good), but the repo also contains `model_int8.onnx`/`model_uint8.onnx` at the same size — an easy wrong pick — and `tokenizer.json` presence in the root was not confirmed by the research. Bundling adds ~120 MB per installer via `bundle.resources: ["resources/*"]` (phase-01:37), which no phase acknowledges, and `resources/*` also ships every OS's PDFium libs into every bundle (no per-target filtering is planned anywhere).
- **Failure scenario:** fetch-models.sh written against a guessed filename fails in CI (`release.yml` step, phase-09:38) or, worse, hashes the wrong variant and passes; installers silently gain ~150 MB of cross-OS dead weight; Phase 3's `#[ignore]` gate (phase-03:77) then tests the wrong quantization.
- **Evidence:** researcher-01-stack-report.md:78/:162; phase-03:26/:54–56; HF tree listing showing 7 ONNX variants incl. three 118 MB files.
- **Fix:** phase-03-diff-alignment.md, Implementation Steps step 3: resolve files via commit-hash URLs (`…/resolve/<commit>/onnx/model_quantized.onnx`), fail loudly if `tokenizer.json` is missing, record byte sizes next to SHA256s, and name the exact variant (`model_quantized.onnx`). Add to phase-01-bootstrap-ci.md Architecture a per-OS resource filtering decision (target-specific `tauri.<os>.conf.json` resource globs) so Linux installers don't carry `pdfium.dll`.

### A6. llama-server ephemeral-port mechanism (`--port 0`) is unverified — MINOR
- **Location:** phase-05-llm-sidecar.md:30, :54 ("`--port 0`-style ephemeral port resolution (parse chosen port from startup log or bind probe)").
- **Flaw:** The plan hedges between two mechanisms without verifying either. llama.cpp's `--port 0` behavior (does the startup log reliably print the chosen port?) is unverified in the research (researcher-01-stack-report.md:85 only specifies `--host 127.0.0.1 --port <random>`).
- **Failure scenario:** `llm_explain` hangs at health polling because the client guesses the wrong port; the 60 s timeout (phase-05:54) turns every explanation attempt into a dead wait with no diagnostic.
- **Evidence:** phase-05:30/:54; researcher-01-stack-report.md:85 (no port-0 verification).
- **Fix:** phase-05-llm-sidecar.md, Implementation Steps step 3: make startup-log parsing the primary mechanism (regex on the logged bind line) and add a unit test with a canned startup log; verify `--port 0` support at implementation time and record it in the manifest notes.

### A7. macOS artifacts are arm64-only; Intel Macs silently unsupported — MINOR
- **Location:** phase-01-bootstrap-ci.md:63 (matrix `macos-14`); phase-09-release.md:38 (`.dmg` list); researcher-01-stack-report.md:84 (`-macos-arm64` asset).
- **Flaw:** `macos-14` runners are Apple Silicon; no x86_64 macOS leg exists, yet nothing in phase-09's README requirements (:59) or acceptance (:31) discloses the supported-architectures matrix.
- **Failure scenario:** An Intel-Mac user downloads the dmg from the v1.0.0 release and gets a binary that won't run; the README's "install instructions (… per-OS notes)" (phase-09:59) has nothing to say about it.
- **Evidence:** phase-01:63; phase-09:38/:59; research:84.
- **Fix:** phase-09-release.md, Implementation Steps step 6: add a "supported platforms" paragraph (macOS arm64-only for v1) to the README requirement, or add a `macos-13`/x64 leg to the release matrix in phase-01:63 if Intel support is desired.

---

## Persona 2 — Failure Mode Analyst

### F1. The core `run_diff` Tauri command and `diff://progress` events are owned by no phase — BLOCKER
- **Location:** `src-tauri/src/commands.rs` first appears as "(modify …)" in phase-05-llm-sidecar.md:45; phases 1–4 create no command file (phase-01:45–46 creates only `main.rs`/`lib.rs` with `ping`); phase-07-frontend.md:53 says commands.rs is "(modify — `run_diff`, `get_diff`, `cancel_diff` … return shapes **finalized**)" — finalizing something never created; phase-06:46 also "modify".
- **Flaw:** The orchestration command that acceptance (d) depends on — `run_diff` running extract → diff → align → enrich with progress events, cancellation, and the `diff://progress` stream the wizard consumes (phase-07:61) — is never planned as an implementation deliverable. Phase 4 builds the pipeline only as a **core test** (phase-04:56), not as a command layer.
- **Failure scenario:** Phase 7 starts with no `run_diff` to "finalize"; the wizard cannot run a diff at all; acceptance (d) has no delivering phase and the plan's phase-by-phase "green" gates never notice because each phase's own criteria test only its own layer.
- **Evidence:** phase-01:45–46 (no commands.rs), phase-05:45 ("modify"), phase-07:53 ("finalize"), phase-04:56 (pipeline as test only); no phase lists `diff://progress` emission as a deliverable.
- **Fix:** Assign ownership: add to phase-04-heuristics.md (natural owner of pipeline glue) a new Related-Code-File + step: create `src-tauri/src/commands.rs` + `src-tauri/src/state.rs` implementing `run_diff`/`get_diff`/`cancel_diff` with `diff://progress` emission and tokio cancellation, with a success criterion that the command round-trips from a headless test. Renumber phase-05/06/07's commands.rs entries to "modify" accordingly.

### F2. Three phases' primary success-criteria commands are invalid cargo invocations — MAJOR
- **Location:** phase-03-diff-alignment.md:72/:77 (`cargo test -p difflens-core align diff`), phase-05-llm-sidecar.md:33/:69/:73 (`… download llm`), phase-08-persistence-reports.md:33/:74 (`… store report`).
- **Flaw:** `cargo test` accepts a single positional `TESTNAME`; a second positional is a clap error ("unexpected argument 'diff' found"). The intended multi-filter form requires `--` (libtest multiple filters, stable since Rust 1.83) or explicit `--test` targets.
- **Failure scenario:** An implementer runs the stated verification command at the end of Phase 3/5/8 and gets a usage error, not a test result — the phase gate as written cannot be executed, so the phase either stalls or is declared green without the documented command.
- **Evidence:** cargo CLI grammar `cargo test [OPTIONS] [TESTNAME] [-- [args]…]`; the six citations above.
- **Fix:** In phase-03:72/:77, phase-05:33/:69/:73, phase-08:33/:74 change to `cargo test -p difflens-core -- align diff` (etc.) or `cargo test -p difflens-core --test align_tests --test diff_tests`. Phase-02:75 (`extract`), 04:69 (`heuristic`), 06:68 (`ocr`) are single-filter and fine.

### F3. CI breaks the moment Phase 2 lands: no PDFium staging in ci.yml — MAJOR
- **Location:** phase-01-bootstrap-ci.md:63 (ci.yml: fmt/clippy/test/build, no fetch steps); phase-02-extraction.md:84 ("loader unit test with `DIFFLENS_PDFIUM_PATH` override **runs on all CI OSes**") and Related Code Files (:39–49) which omit `.github/workflows/ci.yml`; phase-01:80 (`cargo test --workspace` is a standing CI criterion).
- **Flaw:** Phase 2's pdf tests require the bundled pdfium library fetched by `scripts/fetch-pdfium.sh` (:49, gitignored resource), but no phase adds that script to ci.yml. The only ci.yml modifications planned are Phase 5 (egress audit, phase-05:68) and Phase 6 (tesseract, phase-06:57). Ubuntu runners have no system pdfium, so `bind_to_system_library` fallback (:56) also fails.
- **Failure scenario:** Phase 2 pushes; `cargo test --workspace` in CI fails on the loader/pdf tests; the plan's standing criterion "CI green on main" (phase-01:72) is silently violated for the rest of the project, normalizing broken-CI drift.
- **Evidence:** phase-01:63/:80; phase-02:39–49/:84; no ci.yml entry in phase-02's Related Code Files.
- **Fix:** phase-02-extraction.md: add `.github/workflows/ci.yml` (modify) to Related Code Files and a step: run `scripts/fetch-pdfium.sh` before `cargo test` in ci.yml, export `DIFFLENS_PDFIUM_PATH`, and assert the pdf fixture tests actually ran (not skipped) on Linux.

### F4. Phase 1's alpha-tag release references fetch scripts that do not exist yet — MAJOR
- **Location:** phase-01-bootstrap-ci.md:63 ("`release.yml` (… pnpm install + **fetch-scripts hooks** …)") vs phase-02:49 (`fetch-pdfium.sh` created in Phase 2) and phase-03:56 (`fetch-models.sh` created in Phase 3); phase-01:74 (alpha tag is a Phase 1 todo).
- **Flaw:** The Phase 1 release workflow already wires "fetch-scripts hooks", but both scripts arrive in Phases 2–3. At alpha-tag time `release.yml` invokes non-existent scripts and fails — so Phase 1's own verification step (:65) cannot pass.
- **Evidence:** phase-01:63/:65/:74; phase-02:49; phase-03:48/:56.
- **Fix:** phase-01-bootstrap-ci.md, step 8: either create no-op guarded stubs (`scripts/fetch-pdfium.sh`/`fetch-models.sh` that exit 0 with a "not yet implemented" notice, replaced in Phases 2/3) or make the release.yml calls conditional (`[ -f scripts/fetch-pdfium.sh ] && …`). State which in the step text.

### F5. `embeddings-ort` code is never compiled or tested in CI, and Phase 3's "CI runs the ignored ort test on all 3 OSes" claim is implemented by nothing — MAJOR
- **Location:** phase-03-diff-alignment.md:54 (feature "default on for app, **off for fast test runs**"), :83 (Risk Assessment: "CI runs the ignored ort test on all 3 OSes when model artifacts are staged"), phase-01:63 (ci.yml clippy/test with no `--features`), phase-09:38 (release.yml fetches models but runs no tests).
- **Flaw:** Two failures in one: (1) `cargo clippy/test --workspace` in CI never enables `embeddings-ort`, so `OrtE5Embedder` (~the highest-risk code in the project given the RC pin) is compile-checked and tested only inside release builds, where nothing runs tests; (2) the claimed 3-OS ort test has no owner — no phase adds model staging or a feature-enabled test job to any workflow.
- **Failure scenario:** A feature-gated compile error or ort-RC breakage is discovered at the v1.0.0 release build (or by end users), i.e., exactly the failure the `Embedder` trait was built to catch early. The ort RC risk mitigation (plan.md:29, research risk 1) is structurally inert.
- **Evidence:** phase-03:54/:83; phase-01:63; phase-09:38; no ci.yml/release.yml ort job in any phase file.
- **Fix:** phase-03-diff-alignment.md: add a step + related-file entry for `.github/workflows/ci.yml` (modify): a job (can be ubuntu-only) that runs `scripts/fetch-models.sh`, then `cargo clippy --workspace --all-targets --features embeddings-ort -D warnings` and `cargo test -p difflens-core --features embeddings-ort -- --ignored` with the env vars set; stage model artifacts on mac/windows jobs too or mark the claim as ubuntu-only. Phase-01:63's ci.yml description should foreshadow this hook.

### F6. LLM manual gates require a model the plan says the VM cannot run — internal contradiction — MAJOR
- **Location:** phase-05-llm-sidecar.md:75 (success criterion: "Manual gate: install server + model in the dev app, run explain on the DOCX fixture session — tokens stream in EN and VI") vs :82 ("6 GB VM RAM **cannot** run the 4B model locally for dev testing"); repeated at phase-09-release.md:61 (acceptance checklist requires "DOCX/PDF E2E in the real app incl. LLM explanation").
- **Flaw:** Phase 5's own risk assessment invalidates its own success criterion and part of Phase 9's checklist. The documented escape hatch — Qwen2.5-3B fallback (:82) — is not in the manifest (phase-05:52 pins Qwen3-4B + bge-m3 only), so the download manager has no way to install the fallback.
- **Failure scenario:** The implementer reaches the Phase 5 manual gate, cannot run the 4B model on the 6 GB VM, and either skips the gate (v1.0.0 ships with acceptance (d)'s "optional LLM explanation" never verified end-to-end) or improvises a model not covered by the SHA256 manifest — bypassing the app's own integrity guarantee.
- **Evidence:** phase-05:75/:82; phase-09:61; phase-05:52 manifest contents.
- **Fix:** phase-05-llm-sidecar.md: add a Qwen2.5-3B-Instruct Q4_K_M entry (or Qwen3-1.7B) to the manifest with checksum, and reword the manual gate to name the low-RAM model explicitly; or designate an external ≥16 GB machine in the gate text. Phase-09:61 checklist should reference the same model choice.

### F7. All manual gates assume a GUI session on the VM — never stated — MINOR
- **Location:** phase-07-frontend.md:84–86 (manual E2E), phase-05:75 (manual LLM gate), phase-08:76 (manual history/report checks), phase-09:59 (README screenshots), phase-09:57 (tauri-driver uses xvfb — the only place a display strategy appears).
- **Flaw:** `tauri dev`, the wizard, screenshots, and every "manual E2E on the dev VM" criterion need webkit2gtk + a display. The plan never states the VM has one or provides an xvfb fallback for manual (non-CI) verification.
- **Failure scenario:** Phase 7's manual criteria stall on a headless VM; screenshots for acceptance (f) can't be produced; work silently shifts to "I'll check it later".
- **Evidence:** the citations above; no display/xvfb mention outside phase-09:57.
- **Fix:** phase-01-bootstrap-ci.md, `docs/dev-setup.md` step: record the display requirement (GUI session or `xvfb-run` recipe) as part of dev-setup; cross-reference from phase-07:84 and phase-09:59.

### F8. Scanned-PDF fixture generation needs a rasterizer pdf-lib doesn't have — MINOR
- **Location:** phase-02-extraction.md:61 ("generates … one image-only (scanned-like) PDF" via pdf-lib), phase-06-ocr.md:48 ("emit a rasterized scanned-like PDF fixture via pdf-lib + embedded image").
- **Flaw:** pdf-lib draws vector text; it cannot rasterize text into an image. Producing a PNG containing legible text to embed requires node-canvas/satori+resvg/sharp — an unlisted devDependency.
- **Failure scenario:** The fixture step stalls or produces an image-only PDF with no recoverable text, which fails Phase 6's success criterion "OCR-sourced blocks … align" (:73) because there is nothing for Tesseract to read.
- **Evidence:** phase-02:61; phase-06:48; pdf-lib capability set (no rasterization/text-to-PNG).
- **Fix:** phase-02:61 and phase-06:48: name the rasterization dependency explicitly (e.g., `@napi-rs/canvas` or pre-generated PNG assets committed as fixtures) in both fixture-generator entries.

### F9. Phase 7 ships typed wrappers for commands that land in Phase 8 — MINOR
- **Location:** phase-07-frontend.md:58 (`tauri.ts` includes `listSessions`, `deleteSession`, `exportReport` wrappers) vs phase-08:45 (those commands created in Phase 8); phase-07:66 stubs only settings/history **surfaces** behind a feature flag.
- **Flaw:** The invoke wrappers are unconditional in Phase 7 while the commands exist only after Phase 8; the feature-flag stub discipline of :66 is not applied to `tauri.ts`.
- **Failure scenario:** Any accidental invocation before Phase 8 lands is a runtime `command not found` error; worse, the i18n/history UI tests may exercise dead wrappers and pass vacuously.
- **Evidence:** phase-07:58/:66; phase-08:45.
- **Fix:** phase-07:58: mark the three wrappers "added in Phase 8" (move them to phase-08's `tauri.ts` modify entry) or gate them behind the same feature flag.

### F10. Alignment quality has no objective gate beyond two fixture pairs — MINOR
- **Location:** phase-03-diff-alignment.md:62 (tests: "fixture-pair tests (two TXT variants, two DOCX variants → expected ops and moved links)"), :59 (move threshold cosine ≥ 0.85 "default"), :60 (NW gap penalties "tuned by block kind" with no evaluation set).
- **Flaw:** 0.85 and the gap-penalty weights are guesses validated against exactly two hand-made pairs; there is no labeled alignment corpus and no precision/recall criterion, so "alignment works" is unfalsifiable beyond trivia. e5-small cosine on short real-world paragraphs routinely clusters >0.85 for unrelated VN text, so the threshold is plausibly mis-set either way.
- **Failure scenario:** v1.0.0 ships with a move-repair pass that either floods the UI with false `Moved` links or never fires; no phase criterion can detect it because nothing measures it.
- **Evidence:** phase-03:59/:62; no quality-gate criterion in Requirements (:30–35).
- **Fix:** phase-03-diff-alignment.md, Requirements + Success Criteria: add a small hand-labeled fixture set (≥20 gold pairs incl. moves/reorders across the DOCX/PDF fixtures) and a success criterion of measured precision/recall ≥ target on it; document threshold/penalty values in the golden files so changes are reviewable.

---

## Persona 3 — Scope & Complexity Critic

Requested-scope baseline (from the contract summary): 5 formats, 3-pass alignment, heuristics, llama-server explanations, Tesseract OCR (experimental), EN+VI, desktop 3-OS, CI/CD releases, docs. Acceptance criteria (a)–(g) at plan.md:52–60.

### S1. Phase 8 (persistence + HTML/Markdown reports) is outside both the enumerated scope and the acceptance criteria — MAJOR
- **Location:** phase-08-persistence-reports.md entire phase (24 h of 272 h ≈ 9% of effort); plan.md:24 asserts "session history plus HTML/Markdown reports"; plan.md:52–60 — no acceptance criterion (a)–(g) mentions persistence, history, or report export; phase-08's own criteria (:73–76) verify nothing from (a)–(g) except "no regressions".
- **Flaw:** The coordinator's contract enumeration does not include session history or report export. The plan's overview claims the brainstorm contract includes it, but the plan's own authoritative acceptance mapping does not — so the phase's justification rests on an unverifiable assertion while consuming 24 h, a rusqlite dependency, report renderers, golden tests, and two UI pages.
- **Failure scenario:** Two outcomes, both bad: (1) the contract really excludes it → 24 h of unrequested scope shipped in v1.0.0; (2) the contract includes it → the acceptance criteria list is incomplete and Phase 8 can be descoped silently because no (a)–(g) item pins it.
- **Evidence:** plan.md:52–60 (no persistence criterion); phase-08:1–91; contract enumeration in the review brief.
- **Fix:** Either (a) plan.md Requirements: add "(h) session history + HTML/MD export with audit trail" and map it to Phase 8, if the brainstorm contract genuinely includes it, or (b) mark phase-08 as post-1.0 and move its 24 h out of the v1.0.0 plan. Do not leave it ambiguously inside.

### S2. bge-m3 opt-in model is unrequested scope threaded through three phases — MAJOR
- **Location:** plan.md:36 ("Optional downloads … bge-m3 int8"); phase-05-llm-sidecar.md:52 (manifest "optional bge-m3 int8 entry") and :85 (risk section); phase-07-frontend.md:61 (wizard "embedding model select e5-small/bge-m3") and :75 (settings model variant + RAM warning); phase-03:85 (bge-m3 kept "out of this phase's critical path").
- **Flaw:** Nothing in the enumerated scope or acceptance (a)–(g) mentions a second embedding model. bge-m3 adds: a manifest entry with checksum, a download flow, a model-select UI, RAM/latency warnings, and untested interaction with alignment thresholds (phase-03:59's 0.85 cosine was presumably tuned for e5-small — different models invalidate it).
- **Failure scenario:** The bge-m3 path is shipped minimally-tested (it's opt-in everywhere, so every gate exercises e5); users selecting it get a mis-calibrated alignment threshold and a 600 MB+ download — unrequested, underverified scope.
- **Evidence:** plan.md:36; phase-05:52/:85; phase-07:61/:75; phase-03:85; absence from plan.md:52–60.
- **Fix:** Remove bge-m3 from phase-05 manifest, phase-07 wizard/settings, and plan.md:36, or obtain explicit contract confirmation and then add a phase criterion that the alignment gate (F10's labeled corpus) is re-run with bge-m3 vectors.

### S3. ONNX OCR spike bench is code beyond "Tesseract OCR experimental" — MINOR
- **Location:** phase-06-ocr.md:37 (`onnx_spike.rs`), :44, :57 (bench + decision note), :67 (spike todo).
- **Flaw:** The contract scope is Tesseract OCR experimental. The spike adds a feature-gated crate module, a CER harness, and a reports/ artifact — defensible as due diligence, but it is implementation work outside v1 scope and the plan itself defaults it NO-GO (:33).
- **Failure scenario:** 2–4 h of v1 effort spent writing code whose documented purpose is to not be used; the decision note invites re-litigating OCR scope at release time.
- **Evidence:** phase-06:33/:37/:44/:57/:67; research §2.7 verdict (NO-GO, :99).
- **Fix:** phase-06: convert step 6 and the `onnx_spike.rs` related-file entry into a research note only (no product code, no feature), scheduled for v2 planning.

### S4. Auto-updater plumbing exceeds "CI/CD releases" scope — MINOR
- **Location:** phase-01-bootstrap-ci.md:64 (step 9: keypair, GitHub secrets, `plugins.updater` + `createUpdaterArtifacts: true`, `latest.json` endpoint), :73; phase-09:60 (updater endpoint verification).
- **Flaw:** "CI/CD releases" covers artifacts + a GitHub Release. The updater adds a live endpoint contract (`latest.json` must remain resolvable post-publish for updates to work) — an ongoing operational obligation, not a release artifact. Generating keys day 1 is cheap insurance; wiring the updater plugin and endpoint is the creep.
- **Failure scenario:** v1.0.0 ships an updater pointing at a `latest.json` URL the project must keep valid forever; the first re-tag that breaks asset naming silently breaks every installed app's updater.
- **Evidence:** phase-01:64/:73; phase-09:60/:77.
- **Fix:** phase-01:64: keep keypair generation + secrets (cheap, avoids retrofit), but move `plugins.updater` config and endpoint wiring to a post-1.0 phase; phase-09:60/:77 correspondingly drop updater-endpoint verification from the v1.0.0 gate unless the contract explicitly wants auto-update.

### Missing required scope — none found (verification result)
Every acceptance criterion maps to at least one phase success criterion: (a) → phase-02:75/:77, phase-03:72, phase-04:69, phase-06:68, phase-08:74, phase-09:76; (b) → phase-01:70/:79; (c) → phase-01:74/:81, phase-09:71/:77; (d) → phase-02/3/4 (engine), phase-05:75 (LLM), phase-07:84–85 (UI), phase-09:56 (smoke); (e) → phase-07:83–84; (f) → phase-09:79; (g) → phase-05:74 (audit script), phase-09:78. No required deliverable lacks a phase. (But see F1: criterion (d)'s *command layer* is mapped to phases that don't deliver it.)

---

## Persona 4 — Security Adversary

The download manager is correctly designed as the sole egress point (phase-05:59/:68, plan.md:66), SHA256 verification is specified (phase-05:29/:86), TLS via `reqwest rustls-tls` is pinned (phase-01:60), SQL is parameterized-only (phase-08:86), the updater private key is secret-gated with a history grep (phase-01:82). The findings below are the gaps.

### SEC1. HTML report embeds document text with no escaping requirement — stored-XSS-in-export — MAJOR
- **Location:** phase-08-persistence-reports.md:56 (`report/html.rs`: "inline CSS, header …, side-by-side pair table with inline spans, change list with audit-trail appendix" — no escaping mentioned), :57 (markdown.rs: "escape pipes/HTML" — escaping specified only here), :60 (golden tests: "contains change markers + audit rows + inline CSS only" — no hostile-content case).
- **Flaw:** DOCX/PDF/PPTX text is attacker-controlled. Any `<script>`/`<img onerror=…>` payload in a compared document flows through `Change`/`AuditEntry` excerpts into the exported HTML, which the plan designs to "open from disk anywhere" (:25) — i.e., rendered in a browser with file:// context. Only the Markdown renderer gets an escaping requirement.
- **Failure scenario:** A malicious document is diffed, the user exports the HTML report and opens it; injected script executes locally. For a "100% local processing" privacy-branded product (plan.md:74, phase-09:59) this is a credibility-level vulnerability, and no phase criterion would catch it.
- **Evidence:** phase-08:56–57/:60; content path phase-02:37 → phase-03:38 → phase-08:56.
- **Fix:** phase-08-persistence-reports.md, Implementation Steps step 5 and Security Considerations: mandate a single HTML-escape helper applied to **all** document-derived strings (block text, excerpts, audit spans, file names) in `report/html.rs`; add to step 9's golden tests a DOCX fixture containing `<script>`/event-handler payloads asserted escaped in both HTML and Markdown output.

### SEC2. Downloaded-archive extraction hardening (zip-slip, symlinks, entry bombs) unspecified — MAJOR
- **Location:** phase-05-llm-sidecar.md:52 ("archive extraction (single `llama-server` binary into `app_data_dir/bin`)"), :29 (temp-file + atomic rename for the *download*, nothing for *extraction*), :53 (`install()` downloads and extracts); Phase 2's zip limits (phase-02:54) are scoped to document parsing, not this extraction path.
- **Flaw:** llama.cpp archives contain nested paths (`llama-<tag>-bin-<os>/llama-server`). The plan's "single binary" intent is good but names no enforcement: no entry-name allowlist, no rejection of `..`/absolute paths/symlinks/hardlinks, no per-entry decompression cap, no total-cap. The manifest is checksummed (:29/:86), but checksums attest *content integrity*, not *safe extraction* — the attacker model here is a compromised mirror matched by a stale manifest or a checksum collision in the fallback-URL path.
- **Failure scenario (worst case):** A manifest fallback URL (:81) serves a crafted archive whose checksum was never independently pinned; extraction writes outside `app_data_dir/bin` via `../` or a symlink entry → arbitrary file write, then the app *executes* what it extracted at spawn time (:54). Also: a high-compression-ratio archive can exhaust the 6 GB VM disk during install.
- **Evidence:** phase-05:29/:52–54/:81; phase-02:54 (limits exist only for documents).
- **Fix:** phase-05-llm-sidecar.md, Implementation Steps step 2 (and Security Considerations): specify extraction hardening — extract by exact entry allowlist (`**/llama-server`, `**/llama-server.exe`), reject absolute/`..`/symlink/hardlink entries, per-entry and total decompressed-size caps, extract into a staging dir then atomic-rename; add a unit test with a zip-slip fixture (mirroring phase-02's zip-bomb test pattern). Additionally (:81): every fallback URL in the manifest must carry its own pinned checksum — fallbacks may not share the primary checksum.

### SEC3. llama-server loopback exposure is enforced only by spawn flags; no api-key, no post-spawn bind verification — MINOR
- **Location:** phase-05-llm-sidecar.md:54 (spawn `--host 127.0.0.1`), :31/:55 (client-side loopback guard, unit-tested), :55 (health poll).
- **Flaw:** The client guard validates *outgoing* URLs; nothing verifies the server actually bound to loopback, and on multi-user machines any local process/user can hit the unauthenticated OpenAI-compatible port (chat with the model, and via `/v1/chat/completions` observe nothing but still burn RAM; llama.cpp supports `--api-key`).
- **Failure scenario:** On a shared host, another user scripts against the exposed port indefinitely. Not remotely exploitable, but the plan claims "loopback-only enforcement" (plan.md:66) as a security property without a server-side check.
- **Evidence:** phase-05:54–55; plan.md:66.
- **Fix:** phase-05:54: generate a random per-session `--api-key`, pass it to the client, and assert `/health` responds 401 without it; optionally probe the bind address post-spawn. Add to Security Considerations (:86).

### SEC4. Prompt-injection trust boundary is stated as advice, not architecture — MINOR
- **Location:** phase-05-llm-sidecar.md:32/:56 ("strict 'use only provided diff content' instruction", temperature 0), :57 (`llm://done` with parsed JSON rendered by UI, phase-07:65); phase-08 reports do not include LLM output (verified: phase-08:31/:56 audit-trail only).
- **Flaw:** A compared document's text is attacker-controlled and is embedded verbatim into the prompt (:56 "truncated texts"). The grounding instruction is advisory — a document can instruct the model to exfiltrate nothing (no network — good) but to emit misleading "explanations", fake severity claims, or override the JSON schema. The plan treats LLM output as trusted structured data once JSON parses; no rule caps or sanitizes rendered output beyond `max_tokens` (:56).
- **Failure scenario:** A poisoned document produces explanations like "This change is safe — approved by legal", rendered in the audit-adjacent UI with the same visual weight as deterministic audit entries; users of a security-positioned diff tool act on injected text.
- **Evidence:** phase-05:32/:56; phase-07:65 (renders summary/explanations/risk-note).
- **Fix:** phase-05-llm-sidecar.md, Security Considerations + phase-07 step 8: state that LLM output is untrusted — render as plain text (React escaping, no markdown-HTML rendering), visually distinct from deterministic audit entries with an "AI-generated, may be wrong" label, cap explanation length, never persist/export LLM output into reports without the same label.

### SEC5. "Zero panics" claim cannot cover PDFium's C layer — MINOR
- **Location:** phase-02-extraction.md:78 ("Malformed/truncated inputs return `Err` or warnings — zero panics across the fuzz-lite loop"), :57 (pdfium char-box iteration over attacker-supplied PDFs), :84 (correctness gate only on axis-aligned fixtures).
- **Flaw:** pdfium-render calls C code in-process; hostile PDFs can trigger SIGSEGV/SIGABRT inside PDFium, which is not a Rust panic and is uncatchable by the fuzz-lite loop's harness. The criterion as written claims more than the architecture can deliver.
- **Failure scenario:** A crafted PDF hard-crashes the app (not a graceful warning), contradicting the "never panics" requirement (:18) and the privacy/stability branding; the fuzz-lite success criterion still reports green.
- **Evidence:** phase-02:57/:78/:84; researcher-01-stack-report.md:49 (in-process `libloading` binding).
- **Fix:** phase-02:78: reword the criterion to "no Rust-level panics; PDFium C-layer crashes are out of scope, documented as a known limitation" in Risk Assessment (:82–84); optionally pin pdfium-binaries updates as a standing v1.x maintenance note (known-CVE surface of bundled PDFium should appear in phase-09's security considerations).

### SEC6. Manifest integrity test verifies presence, not provenance — MINOR
- **Location:** phase-05-llm-sidecar.md:52 ("a manifest-integrity test asserts every entry has checksum + pinned version"), :80 (pinned b-tag), researcher-01-stack-report.md:146 ("Record model+commit+license in a manifest file bundled with the app").
- **Flaw:** The manifest records checksums but the plan never says *how* the checksums were derived (fetched over TLS from the canonical repo at manifest-authoring time, cross-checked against upstream `SHA256SUMS` where llama.cpp publishes them) — the manifest itself becomes the trust root with no recorded provenance, and the research's recommendation to bundle a model-license/commit manifest (:146) appears nowhere in phase-05/09's artifact lists.
- **Failure scenario:** A typo'd or socially-engineered checksum in a commit "updates" the manifest and the download manager happily verifies attacker content against attacker checksums; license metadata for downloaded models is never shipped, weakening phase-09's attribution requirement (:58 covers bundled/cargo components, not runtime-downloaded ones).
- **Evidence:** phase-05:52/:80; researcher-01-stack-report.md:146; phase-09:58 (no downloaded-artifact license section).
- **Fix:** phase-05:52: require the manifest to record per-entry provenance (source repo, tag/commit, license, checksum-source URL) and have the manifest-integrity test assert those fields; phase-09:58: extend THIRD-PARTY-NOTICES/licences screen with runtime-downloaded components (Qwen3 GGUF, llama-server, optional models).

---

## Cross-Phase Consistency Sweep

Verified consistent: effort totals (24+40+48+24+32+16+48+24+16 = 272 h = plan.md:6); workspace layout `core/` + `src-tauri/` used identically in plan.md:36, phase-01:37, phase-02:37; crate names `difflens` (app) / `difflens-core` (lib) consistent; egress-audit exclusions match plan.md:66 (phase-05:59 vs plan.md:66); sequential same-file modifications (commands.rs, ci.yml, release.yml, generate.mjs, tauri.ts) are ordered, not parallel-ownership conflicts.

Inconsistencies found (beyond F1/BLOCKER, already the biggest):

### C1. plan.md module list omits `store` and `report`; Phase 1 stub list too — MINOR
- **Location:** plan.md:36 (modules: "model, extract, diff, align, heuristic, llm, ocr, download") vs phase-08:41–43 (`core/src/store/*`, `core/src/report/*`) and phase-01:60 (eight stubs only).
- **Fix:** plan.md Architecture + phase-01 step 5: add `store`, `report` to the module enumeration (stubs created in Phase 8).

### C2. Phase 6 silently requires model.rs changes it doesn't declare — MINOR
- **Location:** phase-06:37 ("returns `ExtractedDoc {format: PdfOcr, warnings}`", "style.ocr = true") vs phase-02:37 (`BlockKind`/`ExtractedDoc.format`/`StyleHints` definitions with no OCR variants) and phase-06:40–47 (Related Code Files omit `core/src/model.rs`).
- **Failure scenario:** Phase 6 implementer modifies model.rs informally; Phase 2's model tests/goldens and Phase 7's `types.ts` mirror (phase-07:58) drift from the Rust serde shapes.
- **Fix:** phase-06:41: add `core/src/model.rs` (modify — `Format::PdfOcr`, `StyleHints.ocr`) to Related Code Files and note the Phase 7 TS-mirror update.

### C3. Research ↔ plan runner mismatch (duplicate pointer to A2) — MINOR
- **Location:** researcher-01-stack-report.md:39 ("CI (ubuntu-24.04 runner)") vs phase-01:37/:63 (ubuntu-22.04). The plan contradicts its own authoritative research link (plan.md:19 declares the research "authoritative").
- **Fix:** resolved by A2's fix; also correct phase-01's Key Insight (:22) which discusses distro webkit versions without naming the runner it targets.

---

## Verdict

| # | Persona | Findings | BLOCKER | MAJOR | MINOR |
|---|---------|----------|---------|-------|-------|
| 1 | Assumptions Destroyer | 7 | 2 (A1, A2) | 3 (A3, A4, A5) | 2 (A6, A7) |
| 2 | Failure Mode Analyst | 10 | 1 (F1) | 5 (F2–F6) | 4 (F7–F10) |
| 3 | Scope & Complexity Critic | 4 | 0 | 2 (S1, S2) | 2 (S3, S4) |
| 4 | Security Adversary | 6 | 0 | 2 (SEC1, SEC2) | 4 (SEC3–SEC6) |
| 5 | Consistency sweep | 3 | 0 | 0 | 3 (C1–C3) |
| | **Total** | **30** | **3** | **12** | **13** |

### Blockers (must fix before implementation begins)
1. **A1** — `tauri-apps/tauri-action@v2` does not exist (real line is `@v0`, v0.6.x); release pipeline fails on first use → phase-01:63.
2. **A2** — CI pinned to `ubuntu-22.04`, deprecated since 2026-09-17 (unsupported 2027-04-17) and contradicting the authoritative research's ubuntu-24.04 → phase-01:37/:63.
3. **F1** — `run_diff` command + `diff://progress` emitter + `src-tauri/src/commands.rs` creation are owned by no phase; acceptance (d)'s application path is missing → assign to Phase 4.

### Overall assessment
NOT ready for implementation. The plan is unusually strong on architecture, determinism gates, and egress design, but it (1) hard-pins two externally broken infrastructure facts, (2) leaves the central Tauri command unowned, and (3) has a systematic verification gap: feature-gated ort code and PDFium-dependent tests are never exercised by the CI it defines. All three blockers plus the 12 MAJORs are plan-text fixes (no re-architecture required); recommend applying fixes and re-running `/ak-plan validate` before cooking.

---

## Resolution (2026-11-17 fix pass)

All 3 BLOCKER, 12 MAJOR and 13 MINOR findings applied to the plan files; `ak plan validate` passes afterward.

- **A1** — phase-01: `tauri-action@v2` → `tauri-apps/tauri-action@v0` in Architecture + step 8, with implementation-time "resolve current major tag / prefer commit SHA" note; Risk Assessment records the UNVERIFIED→resolved history.
- **A2** — All workflows pinned `ubuntu-24.04` (phase-01 Architecture + step 8); release-time runner-label re-verification note incl. `macos-14` vintage; Risk Assessment documents the 2026-09-17 deprecation; Key Insight names the runner.
- **A3** — phase-03: ORT linking decided (static via ort default `download-binaries`, no runtime lib to bundle); phase-09 Architecture + step 7 add per-OS packaged-app release smoke (e5 model loads in the built app) with `load-dynamic` fallback bundling path.
- **A4** — phase-01 step 2: measured disk-budget table (rustup/registry/node_modules/target/ORT/resources), mount chosen by `df`/`du` measurement, <2 GB-headroom abort criterion (drop `embeddings-ort` locally, rely on Phase 3 CI ort job).
- **A5** — phase-03 step 3: commit-hash URLs, exact `onnx/model_quantized.onnx` variant (≈118 MB, vs `model_int8`/`model_uint8` look-alikes), byte sizes + SHA256 recorded, loud failure on missing `tokenizer.json`; phase-01 Architecture + Related Files add per-OS `tauri.<os>.conf.json` resource globs.
- **A6** — phase-05 step 3: startup-log bind-line parsing is the primary ephemeral-port mechanism with a canned-log unit test; `--port 0` verified + recorded at implementation.
- **A7** — phase-09 step 6: README supported-platforms paragraph (macOS arm64-only for v1); Requirements + success criteria reference the matrix.
- **F1** — Ownership assigned to phase-03 (diff engine, per coordinator direction): phase-03 Related Files + step 11 create `src-tauri/src/commands.rs` + `src-tauri/src/state.rs` with `run_diff`/`get_diff`/`cancel_diff`, `diff://progress` emission and tokio cancellation + headless round-trip criterion; phase-04 wires enrich (modify), phase-05/06/07/08 commands.rs + state.rs entries are modify-only.
- **F2** — phase-03/05/08: `cargo test -p difflens-core -- align diff` / `-- download llm` / `-- store report` in Requirements, Todo, Success Criteria (single-filter phase-02/04/06 commands left as-is, valid).
- **F3** — phase-02: `.github/workflows/ci.yml` added to Related Files; step 11 stages pdfium via `fetch-pdfium.sh`, exports `DIFFLENS_PDFIUM_PATH`, asserts pdf tests ran (not skipped) on Linux.
- **F4** — phase-01 step 8: release.yml fetch-script calls are guarded (`if [ -f scripts/fetch-pdfium.sh ]; then …`) since the scripts arrive in Phases 2–3.
- **F5** — phase-03 step 12 + Related Files: ubuntu-only CI job runs `fetch-models.sh` then `cargo clippy --workspace --all-targets --features embeddings-ort -- -D warnings` and `cargo test -p difflens-core --features embeddings-ort -- --ignored`; Risk Assessment claim corrected to ubuntu-only (mac/windows via Phase 9 packaged-app smoke); phase-01 ci.yml description foreshadows the hook.
- **F6** — phase-05: Qwen2.5-3B-Instruct Q4_K_M low-RAM fallback entry added to the manifest (UNVERIFIED checksum flagged for implementation-time verification); manual gate names it for the 6 GB VM (or Qwen3-4B on ≥16 GB); phase-09 checklist uses the same manifest-pinned model.
- **F7** — phase-01 step 1 records the display requirement (GUI session or `xvfb-run`) in `docs/dev-setup.md`; phase-07 manual E2E criterion and phase-09 checklist cross-reference it.
- **F8** — phase-02 step 9 + phase-06 Related Files: rasterization dependency named explicitly (`@napi-rs/canvas`, or committed pre-rendered PNG fixtures) — pdf-lib cannot rasterize text.
- **F9** — phase-07 step 1 drops `listSessions`/`deleteSession`/`exportReport` wrappers (explicitly deferred); phase-08 Related Files + step 8 add them to `tauri.ts`.
- **F10** — phase-03 Requirements/step 10/Success Criteria: ≥20 hand-labeled gold pairs (incl. moves/reorders) in `core/tests/align_golden/`, precision ≥ 0.85 / recall ≥ 0.80 gate on move detection, thresholds + gap penalties recorded in golden files.
- **S1** — plan.md Requirements adds "(h) session history + HTML/Markdown report export with audit trail → Phase 8"; phase-09 closes (h) and its checklist covers it.
- **S2** — bge-m3 kept strictly opt-in download-only, never bundled: manifest entry gated on re-running the Phase 3 gold alignment set with bge-m3 vectors (0.85 threshold is e5-calibrated); plan.md, phase-05, phase-07 wizard/settings annotated accordingly.
- **S3** — phase-06: ONNX OCR spike converted to a research note only (`reports/ocr-onnx-spike-research.md`, no product code, no cargo feature), default NO-GO, v2 planning.
- **S4** — phase-01 keeps keypair + secrets but drops `plugins.updater`/`createUpdaterArtifacts`/endpoint wiring (post-1.0); phase-09 requirement/success criteria drop updater artifacts + endpoint verification from the v1.0.0 gate.
- **SEC1** — phase-08 step 5 + Security: single `html_escape()` helper on all document-derived strings in `report/html.rs`; step 9 + success criteria add the hostile-content (`<script>`/`onerror=`) golden test for HTML and Markdown.
- **SEC2** — phase-05 step 2: entry allowlist (`**/llama-server`/`.exe`), rejection of absolute/`..`/symlink/hardlink entries, per-entry + total size caps, staging dir + atomic rename, zip-slip unit tests; every fallback URL carries its own pinned checksum.
- **SEC3** — phase-05 step 3: random per-session `--api-key`, `/health` asserts 401 without it, post-spawn bind verification; Security Considerations updated.
- **SEC4** — phase-05 Security + phase-07 step 8: LLM output treated as untrusted — plain-text React-escaped rendering (no markdown-HTML), length cap, "AI-generated, may be wrong" label, never persisted/exported without the label; reports exclude LLM output.
- **SEC5** — phase-02 success criterion reworded to "no Rust-level panics" with PDFium C-layer crashes documented as a known limitation; phase-09 security/risk add the bundled-PDFium CVE re-check at each release.
- **SEC6** — phase-05 manifest records per-entry provenance (repo, tag/commit, license, checksum-source URL) asserted by the integrity test; phase-09 step 5 adds a runtime-downloaded components section (llama-server, Qwen3-4B, Qwen2.5-3B fallback, optional bge-m3) to notices + Licenses screen.
- **C1** — plan.md Architecture + phase-01 step 5 enumerate `store`/`report` (stubs added in Phase 8).
- **C2** — phase-06 Related Files adds `core/src/model.rs` (modify — `Format::PdfOcr`, `StyleHints.ocr`) with a Phase 7 TS-mirror note (phase-07 step 1 records it).
- **C3** — Resolved by A2 (`ubuntu-24.04` everywhere); phase-01 Key Insight now names the runner explicitly.
