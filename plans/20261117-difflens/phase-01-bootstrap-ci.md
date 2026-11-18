---
title: "Phase 1: Repo bootstrap, toolchain, Tauri skeleton, CI/CD"
status: todo
priority: P1
effort: 24h
---

# Phase 1: Repo Bootstrap, Toolchain, Tauri Skeleton, CI/CD

## Context Links

- Plan: [plan.md](./plan.md)
- Research: [research/researcher-01-stack-report.md](./research/researcher-01-stack-report.md) §2.1 (Tauri deps, tauri-action, sidecar API, disk usage), §3 risks 3 and 5.

## Overview

**Priority:** P1 · **Status:** pending · **Effort:** ~24h
Bootstrap the public GitHub repo (`vibecoder880/difflens`, MIT), install the Rust toolchain and Linux webkit2gtk-4.1 build deps on the dev VM, scaffold the Tauri 2 + React/Vite/Tailwind/shadcn frontend with the Cargo workspace (`src-tauri` thin shell + `core` headless crate), set up disk hygiene for the 9.5 GB-free VM, and stand up CI (`ci.yml`) plus the 3-OS release pipeline (`release.yml` with tauri-action and updater signing keys), verified end-to-end by an alpha tag.

## Key Insights

- Tauri 2 needs webkit2gtk-4.1 (not 4.0) on Linux; Ubuntu 24.04 (the pinned CI Linux runner and dev distro) dropped 4.0, so the Tauri 2 line is the only viable target.
- tauri-action builds all 3 OSes from one matrix. Its release/tag line is `v0` (v0.6.x at planning time; the research flagged the exact tag UNVERIFIED) — resolve the current major tag at implementation time and prefer pinning by commit SHA. Without updater secrets it still uploads unsigned artifacts.
- A Tauri `target/` reaches 4–8 GB in debug (10–20 GB with release artifacts); the VM has 9.5 GB free → measure each mount first (`df -h`/`du`), relocate `CARGO_TARGET_DIR` to the measured-largest mount, keep local builds debug-only, and if <2 GB headroom remains after the first debug build, drop `embeddings-ort` from local dev builds (ort verification then rides the Phase 3 CI job).
- Updater signing keys are free (`tauri signer generate`) and are generated + stored day 1 to avoid retrofitting; the `plugins.updater` config and `latest.json` endpoint wiring itself is post-1.0 scope, not v1.

## Requirements

- [ ] Rust stable via rustup (minimal profile); Linux build deps installed; pnpm available via npm.
- [ ] `cargo build` (debug, Linux) and `pnpm build` green on the VM; `ping` command round-trips from React.
- [ ] Public GitHub repo, MIT LICENSE, `.gitignore` sized for the disk constraint, CI green on main.
- [ ] Updater keypair generated; private key stored as GitHub secret only, never committed.
- [ ] Alpha tag produces a draft GitHub Release with Windows/macOS/Linux artifacts.

## Architecture

Repo root = Vite frontend + Cargo workspace. Root `Cargo.toml` with `members = ["src-tauri", "core"]`, resolver 2, shared `[workspace.dependencies]` version pins. `src-tauri/` is the tauri 2.12.x app shell (identifier `app.difflens.desktop`, capabilities, `bundle.resources: ["resources/*"]` pre-created for Phase 3 models and Phase 2 PDFium libs). Per-OS resource filtering decided here: target-specific `tauri.linux.conf.json` / `tauri.macos.conf.json` / `tauri.windows.conf.json` override `bundle.resources` globs so each installer carries only its own OS's PDFium library and model files (no cross-OS dead weight). `core/` is the `difflens-core` lib crate with module stubs (`model`, `extract`, `diff`, `align`, `heuristic`, `llm`, `ocr`, `download`; `store`/`report` stubs added in Phase 8). Workflows: `ci.yml` (fmt/clippy/test/build on ubuntu-24.04; Phases 2–3 append a PDFium staging step and a feature-gated `embeddings-ort` job) and `release.yml` (tauri-action matrix `ubuntu-24.04` / `macos-14` / `windows-latest` — re-verify runner labels at each release, artifact upload, draft release on `v*` tags, using `tauri-apps/tauri-action@v0`).

## Related Code Files

- `Cargo.toml` (create — workspace root with shared dependency pins)
- `src-tauri/Cargo.toml`, `src-tauri/build.rs` (create — Tauri app crate)
- `src-tauri/tauri.conf.json` (create — window, bundle, resources)
- `src-tauri/tauri.linux.conf.json`, `src-tauri/tauri.macos.conf.json`, `src-tauri/tauri.windows.conf.json` (create — per-OS `bundle.resources` globs so installers carry only their own OS's PDFium/model files)
- `src-tauri/capabilities/default.json` (create)
- `src-tauri/src/main.rs`, `src-tauri/src/lib.rs` (create — thin shell + `ping`)
- `src-tauri/resources/.gitkeep` (create — bundle resources dir)
- `core/Cargo.toml`, `core/src/lib.rs` (create — `difflens-core` crate + module stubs)
- `package.json`, `vite.config.ts`, `tsconfig.json`, `index.html` (create)
- `src/main.tsx`, `src/App.tsx`, `src/styles/globals.css` (create)
- `.gitignore`, `LICENSE`, `README.md` (stub), `docs/dev-setup.md` (create)
- `scripts/setup-linux-deps.sh`, `scripts/disk-hygiene.sh` (create)
- `.github/workflows/ci.yml`, `.github/workflows/release.yml` (create)

## Implementation Steps

1. Install toolchain on the VM: rustup with minimal profile + stable toolchain (`rustup-init -y --profile minimal`); `npm i -g pnpm`; create and run `scripts/setup-linux-deps.sh` installing `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`; assert `pkg-config --modversion webkit2gtk-4.1` succeeds; record versions and the display requirement for later manual gates (GUI session or `xvfb-run` recipe for webkit2gtk apps — Phase 7/9 manual E2E depend on it) in `docs/dev-setup.md`.
2. Disk hygiene (measured, not assumed): run `df -h` per mount and record an itemized budget in `docs/dev-setup.md` — rustup minimal ≈1 GB; cargo registry ≈1–1.5 GB; `node_modules` + pnpm store ≈0.5–1 GB; Tauri debug `target/` ≈4–8 GB; ORT build download ≈0.3 GB; pdfium + e5-small resources ≈0.15 GB. Verify `$HOME/.cache` is actually on a different/larger mount than the 9.5 GB filesystem before choosing the `CARGO_TARGET_DIR` relocation target — `scripts/disk-hygiene.sh` selects the measured-largest mount and exports `CARGO_TARGET_DIR` + `pnpm config set store-dir` accordingly. Abort/escalation criterion: if <2 GB headroom remains after the first debug build, drop `embeddings-ort` from local dev builds (feature off locally) and rely on the Phase 3 CI ort job for ort-featured verification; document a cargo-clean cadence (clean between debug/release cycles; before any new large dependency set); `.gitignore` covers `target/`, `src-tauri/target/`, `dist/`, `node_modules/`, `src-tauri/resources/models/`, `src-tauri/resources/pdfium/`, `*.gguf`, `*.key`.
3. Scaffold frontend: Vite 7 React-TS template; Tailwind 4 via `@tailwindcss/vite` plugin with CSS-first `@theme` tokens (no tailwind.config); `shadcn` init with the Radix primitives flag (research §2.9 notes the Base-UI default drift — pin Radix explicitly); add TanStack Router, TanStack Query, TanStack Virtual, i18next 27 + react-i18next deps.
4. Scaffold Tauri: `pnpm tauri init`; pin `tauri = "2.12"`, `tauri-build = "2.12"`, plugins `shell`, `dialog`, `fs`, `opener`; `tauri.conf.json` with identifier, `build.frontendDist: "../dist"`, `build.devUrl: "http://localhost:5173"`, `bundle.resources: ["resources/*"]`.
5. Create root workspace `Cargo.toml` with `[workspace.dependencies]` pins (tauri 2.12; zip 9.7; roxmltree 0.21; calamine 0.38; pdfium-render 0.9.5; similar 3.2; ort `=2.0.0-rc.15`; tokenizers 0.23; rusqlite `bundled`; serde/serde_json; thiserror; tokio; reqwest rustls-tls) and the `core/` crate with `lib.rs` declaring the eight module stubs (`store`/`report` stubs are added by Phase 8), each compiling empty.
6. Implement `ping` command in `src-tauri/src/lib.rs`; call it from `App.tsx` on mount and render the response to prove the Tauri↔React bridge.
7. Write MIT `LICENSE` full text; README stub (name, one-liner, license badge); note `THIRD-PARTY-NOTICES.md` placeholder for Phase 9.
8. `git init`, initial commit; `gh repo create vibecoder880/difflens --public --source . --push`; add `ci.yml` (checkout, pnpm + rust caches, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm build`, `ubuntu-24.04` with `jlumbroso/free-disk-space`; later phases append Phase 2's PDFium staging step and Phase 3's feature-gated `embeddings-ort` job) and `release.yml` (matrix `ubuntu-24.04`/`macos-14`/`windows-latest` — re-verify runner labels, incl. the `macos-14` vintage, at each release; pnpm install + conditional fetch-script hooks, `if [ -f scripts/fetch-pdfium.sh ]; then ./scripts/fetch-pdfium.sh; fi` and likewise `fetch-models.sh`, since those scripts only arrive in Phases 2–3; `tauri-apps/tauri-action@v0` — the `v0` rolling tag, v0.6.x line at planning time; resolve the current major tag at implementation and prefer pinning by commit SHA — with `tagName: v__VERSION__`, `releaseDraft: true`, `includeRelease: true`, per-OS artifact upload, free-disk-space on Linux).
9. `pnpm tauri signer generate -w ~/.tauri/difflens.key`; add `TAURI_SIGNING_PRIVATE_KEY` and password as GitHub secrets for later use. Do NOT wire `plugins.updater` / `createUpdaterArtifacts: true` in v1 — auto-update is post-1.0 scope; the keypair + secrets are stored now to avoid retrofitting.
10. Verify pipeline: push main → CI green; push tag `v0.1.0-alpha.0` → release workflow → confirm draft release lists Windows (.msi/.exe), macOS (.dmg/.app.tar.gz), Linux (.deb/.AppImage) artifacts; run local `cargo clean` and `df -h` to confirm ≥5 GB free after the debug build.

## Todo

- [ ] rustup + webkit2gtk-4.1 deps + pnpm installed; `docs/dev-setup.md` written
- [ ] Workspace compiles: `cargo build` and `pnpm build` green locally
- [ ] `ping` command round-trips from React
- [ ] Repo public, LICENSE, .gitignore committed; CI green on main
- [ ] Updater keys in GitHub secrets (updater plugin config + `latest.json` endpoint deferred post-1.0)
- [ ] Tag `v0.1.0-alpha.0` → draft release with 3-OS artifacts
- [ ] Post-build disk check: ≥5 GB free (`df -h`)

## Success Criteria

- `cargo build -p difflens` exits 0 on the VM (debug profile); `pnpm build` exits 0.
- `cargo test --workspace` exits 0.
- CI run on main is green (`gh run list --limit 1` shows success); tag `v0.1.0-alpha.0` yields a draft release with installers for all 3 OSes.
- `git grep -l "PRIVATE KEY" $(git rev-list --all)` returns nothing (no secrets committed).

## Risk Assessment

- webkit2gtk version mismatch on the VM distro → setup script asserts `pkg-config --modversion webkit2gtk-4.1` and fails with the exact missing package list.
- tauri-action tag risk → the research flagged the exact tag UNVERIFIED and the plan initially pinned a nonexistent `@v2`; resolved by live check: the release line is `@v0` (v0.6.x at planning). Verify the current major tag / commit SHA at implementation before the first tag push.
- Runner deprecation risk → ubuntu-22.04 entered deprecation 2026-09-17 (unsupported 2027-04-17), so all workflows pin `ubuntu-24.04` (matching the authoritative research); re-verify runner labels (incl. `macos-14` vintage) at each release.
- VM disk exhaustion → itemized, measured budget from step 2 (select the `CARGO_TARGET_DIR` mount by measurement, not assumption); if <2 GB headroom remains after the first debug build, drop `embeddings-ort` from local dev builds and rely on CI for ort-featured verification.
- GH Linux runner disk exhaustion → `jlumbroso/free-disk-space` + never cache `target/` (cache `~/.cargo` registry only).
- Updater secret misconfiguration → auto-update wiring is post-1.0 scope; the generated keypair + secrets sit unused until then, so a misconfiguration cannot break the v1 release (release.yml uploads plain artifacts).

## Security Considerations

- Signing private key lives only in GitHub secrets and local `~/.tauri`; `.gitignore` blocks `*.key`.
- GitHub Actions pinned to major version tags; no third-party actions beyond tauri-action, free-disk-space, and official setup actions.

## Next Steps

Phase 2 lands the extraction layer in `core/src/extract/` with fixtures under `core/tests/fixtures/` and PDFium binaries under `src-tauri/resources/pdfium/` (fetched by script, gitignored).
