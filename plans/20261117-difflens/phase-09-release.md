---
title: "Phase 9: Release engineering (icons, 3-OS pipeline, E2E smoke, README, v1.0.0)"
status: todo
priority: P1
effort: 16h
---

# Phase 9: Release Engineering

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.1 (tauri-action, updater, unsigned-distribution friction), §2.10 (licensing audit + attribution actions), §3 risk 5.

## Overview

**Priority:** P1 · **Status:** pending · **Effort:** ~16h
Finalize production release engineering: app icons, final app metadata/version 1.0.0, the 3-OS CI release pipeline verified end-to-end, a core E2E smoke test over the full pipeline, README with build instructions + screenshots, third-party attribution, and the `v1.0.0` tag publishing the GitHub Release. Auto-update wiring is post-1.0 scope (Phase 1 stored the signing keys; no updater artifacts ship in v1). This phase closes acceptance criteria (c), (f), (h), and the verification half of (a), (b), (g).

## Key Insights

- Updater plugin wiring is deferred to post-1.0 (keeps v1 scope to "CI/CD releases"): Phase 1 generated and stored the signing keys as GitHub secrets, but v1.0.0 ships no updater artifacts; release verification covers installers only, and the keypair is reused unchanged when auto-update lands later.
- Unsigned macOS/Windows builds hit Gatekeeper/SmartScreen: acceptable for OSS v1, documented ("right-click → Open"); Apple Developer ID deferred.
- Attribution obligations: PDFium BSD-3 notice, Apache-2.0 NOTICE for `similar` + `tokenizers`, model licenses (e5-small MIT, Qwen3 Apache-2.0), llama.cpp MIT, Tesseract Apache-2.0 — `cargo about generate` covers crates, manual sections cover binaries/models.
- The core E2E smoke (DOCX pair + PDF pair through extract→diff→align→heuristic→report JSON) is the automatable core of acceptance (d); UI-level acceptance is a scripted manual checklist plus an optional tauri-driver smoke.

## Requirements

- [ ] App icon set generated; final bundle metadata (productName, version 1.0.0, per-OS bundle targets).
- [ ] Core E2E smoke test green (`core/tests/e2e_test.rs`).
- [ ] Tag push produces a draft release with installers for 3 OSes (updater artifacts `latest.json`/`.sig` are post-1.0).
- [ ] README complete: build instructions, screenshots, supported-platforms matrix, privacy statement, license, attribution link.
- [ ] `THIRD-PARTY-NOTICES.md` complete (bundled + runtime-downloaded components); in-app licenses screen lists them.
- [ ] Acceptance checklist (a)–(h) verified and recorded.

## Architecture

Release artifacts: Windows `.msi` + NSIS `.exe`, macOS `.dmg` (arm64), Linux `.deb` + `.AppImage`. `release.yml` runs `scripts/fetch-models.sh` + `scripts/fetch-pdfium.sh` before tauri-action so bundled resources are present per target. ONNX Runtime is statically linked (ort's default `download-binaries` feature), so no runtime ORT shared library needs bundling; if any target ever switches to `load-dynamic`, this phase adds per-OS ORT resource bundling plus the release smoke below covers it. Release smoke: the packaged app loads the e5 model and initializes embeddings on each OS (scripted where possible, else a release-checklist item). E2E smoke lives in `core/tests/e2e_test.rs` using committed fixtures only. Notices are generated (`cargo about`) + curated; the about screen renders the same file.

## Related Code Files

- `src-tauri/icons/*` (create via `pnpm tauri icon`), `assets/icon.png` (create — 1024px source)
- `src-tauri/tauri.conf.json` (modify — version 1.0.0, productName DiffLens, bundle targets)
- `package.json`, `core/Cargo.toml`, `src-tauri/Cargo.toml` (modify — version 1.0.0 sync)
- `core/tests/e2e_test.rs` (create)
- `THIRD-PARTY-NOTICES.md`, `scripts/gen-notices.sh` (create)
- `src/components/about/LicensesScreen.tsx` (create — renders notices + model licenses)
- `README.md`, `docs/build.md`, `docs/privacy.md` (modify/create)
- `.github/workflows/release.yml` (modify — final verification pass)
- `scripts/release-checklist.md` (create — manual UI acceptance script)

## Implementation Steps

1. Design a 1024px source icon (SVG→PNG) and run `pnpm tauri icon assets/icon.png` to generate all platform icon sets into `src-tauri/icons/`.
2. Finalize `tauri.conf.json`: version 1.0.0, productName DiffLens, identifier, per-OS bundle targets (`msi`, `nsis` / `dmg` / `deb`, `appimage`), shortDescription, longDescription; sync versions in `package.json` + Cargo.tomls.
3. Core E2E smoke `core/tests/e2e_test.rs`: DOCX fixture pair + PDF fixture pair through `extract → diff → align → heuristic enrich → report render`; assert pair coverage, ≥1 severity-banded change, non-empty audit trail, deterministic JSON, valid HTML/Markdown output.
4. Optional UI smoke: minimal tauri-driver + WebdriverIO spec (launch app, assert window title + wizard visible) on ubuntu with xvfb in a separate CI job marked non-blocking; if flaky in CI, the manual checklist remains the gate (document decision).
5. Attribution: `scripts/gen-notices.sh` runs `cargo about generate` for crates; manually append PDFium BSD-3, ONNX Runtime MIT, e5-small MIT (base model card + pinned export commit), llama.cpp MIT, Tesseract Apache-2.0, `similar`/`tokenizers` Apache-2.0 NOTICE text, plus a runtime-downloaded components section with licenses + pinned commits/tags (llama-server binary, Qwen3-4B GGUF, Qwen2.5-3B low-RAM fallback GGUF, optional bge-m3 int8); produce `THIRD-PARTY-NOTICES.md`; add the in-app Licenses screen rendering it.
6. README: hero screenshot (Linux light + dark), feature list, 100%-local privacy statement (link `docs/privacy.md`), supported-platforms paragraph (Windows x64, Linux x64 deb/AppImage, macOS Apple Silicon arm64 only for v1 — Intel Macs unsupported in v1), install instructions (GitHub Releases + per-OS notes incl. macOS right-click-open and Windows SmartScreen note), build-from-source (Linux deps via script, rustup, pnpm, fetch scripts), optional components (LLM model — Qwen3-4B or the Qwen2.5-3B low-RAM manifest entry, OCR Tesseract) with size/RAM guidance, license + attribution link; `docs/build.md` with detailed dev setup incl. disk-hygiene notes.
7. Release pipeline verification: push tag `v1.0.0-rc.1` → confirm draft release assets (6+ installers) and run the per-OS packaged-app smoke (app launches, e5 model loads, embeddings initialize); updater artifacts/endpoint are post-1.0 scope; fix and re-tag if needed.
8. Manual acceptance script `scripts/release-checklist.md`: run the (a)–(h) checklist — cargo test, local builds, 3-OS release, DOCX/PDF E2E in the real app incl. LLM explanation using the manifest's low-RAM Qwen2.5-3B entry on the dev VM (or Qwen3-4B on a ≥16 GB machine), i18n switch, session history + report export, README completeness, egress audit; record results in the release notes.
9. Final release: publish the draft after checklist passes, tag `v1.0.0`, bilingual (EN+VI) release notes summarizing features + honest OCR experimental note.

## Todo

- [ ] Icon set generated from source icon
- [ ] Version 1.0.0 synced across configs
- [ ] Core E2E smoke test green
- [ ] THIRD-PARTY-NOTICES.md (incl. runtime-downloaded components) + Licenses screen complete
- [ ] README + docs/build.md + docs/privacy.md complete with screenshots + supported-platforms matrix
- [ ] `v1.0.0-rc.1` draft release verified (installers + per-OS packaged-app smoke; updater artifacts post-1.0)
- [ ] Acceptance (a)–(h) checklist recorded; `v1.0.0` published

## Success Criteria

- `cargo test --workspace` exits 0 including `e2e_test.rs`.
- `gh release view v1.0.0` lists installers for Windows/macOS/Linux (updater artifacts `latest.json`/`.sig` are post-1.0 scope).
- `scripts/audit-egress.sh` exits 0; `git grep -iE "telemetry|analytics" src core src-tauri` returns nothing.
- README on GitHub renders with screenshots, supported platforms, build steps, license and attribution links.

## Risk Assessment

- Auto-update deferred post-1.0 → no updater artifacts or endpoint contract to verify in v1; the Phase 1 keypair + secrets are stored and documented for the post-1.0 wiring, avoiding retrofit cost.
- macOS unsigned-download friction → README + release notes instructions; acceptance unaffected (draft release criterion). macOS arm64-only for v1 is disclosed in the README supported-platforms paragraph.
- cargo-about license detection gaps → manual sections guarantee the audit-table obligations are met regardless.
- PDFium known-CVE surface (bundled C library) → pdfium-binaries version pinned in fetch script; re-check for CVE advisories at each release; C-layer crashes on hostile PDFs remain a documented known limitation (Phase 2).

## Security Considerations

- Auto-update is not wired in v1: no updater endpoint to keep alive and no `latest.json` contract; the signing private key still lives only in GitHub secrets.
- Release artifacts built solely in CI from clean checkouts; no local-built binaries uploaded.
- Manifest pins (models/llama.cpp) re-verified at release time; checksum mismatch fails the build; bundled PDFium CVE surface re-checked at each release.

## Next Steps

Project v1.0.0 complete. Post-release: triage user issues, collect OCR quality feedback (v2 ONNX decision), consider bge-m3 default tuning and code-signing certificates.
