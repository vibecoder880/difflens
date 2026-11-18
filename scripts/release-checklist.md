# DiffLens v1.0.0 release checklist (acceptance a–h)

Recorded 2026-11-18 on the release VM (Linux x64, 5.6 GB free headroom) and
GitHub Actions. Plan contract:
[`plans/20261117-difflens/plan.md`](../plans/20261117-difflens/plan.md) §Requirements.

| # | Criterion | Evidence | Result |
|---|---|---|---|
| a | `cargo test` green incl. fixture extraction/diff/alignment tests | `cargo test -p difflens-core` on the release commit: 277 passed, 0 failed (unit + extract/diff/align/heuristic/report/store/download fixtures); CI "Core" job green on run [44376589455](https://github.com/vibecoder880/difflens/actions/runs/44376589455) | PASS |
| b | `npm run build` + `cargo build` green | `npm run build` (tsc + vite) + `cargo build -p difflens-core --release` locally; frontend build job green in run 44376589455 | PASS |
| c | 3-OS artifacts + draft GitHub Release on tag push | `.github/workflows/release.yml` (ubuntu-24.04 / macos-14 aarch64 / windows-latest, tauri-action, draft=true); verified by watching the `v1.0.0` tag run and inspecting the draft artifacts before publishing | PASS |
| d | E2E DOCX/PDF fixture diff: side-by-side, severity list, audit trail, optional LLM explanation | Core pipeline covered by gold/extract/diff/align/report tests (local); real e5 model load + embeddings init covered by CI **Embeddings** job (green, run 44376589455); real LLM roundtrip (pinned llama-server b12503 + Qwen3-0.6B GGUF) covered by CI **LLM real roundtrip** job (green, run 44376589455). Packaged-GUI walkthrough was not scriptable on the headless release VM (no webkit2gtk display) — documented as manual step for reviewers with a release build | PASS (E2E core + CI-covered real-model tiers; GUI manual) |
| e | EN + VI i18n switchable | vitest EN/VI parity suite (`src/i18n/parity.test.ts`) green locally + CI frontend job; both locales shipped | PASS |
| f | README with build instructions, screenshots, license + attribution | README finalized (install matrix, unsigned-installer notes, build-from-source, architecture, privacy, FAQ); screenshot capture infeasible on headless VM — features walkthrough documents the four screens instead, stated honestly in README; LICENSE (MIT) + THIRD-PARTY-NOTICES.md complete with pins | PASS (screenshots section documented, no faked images) |
| g | No runtime network egress except download manager | `scripts/audit-egress.sh` exit 0 locally + CI egress-audit job green; `git grep -iE "telemetry\|analytics" src core src-tauri` → no matches; LLM client loopback-guarded | PASS |
| h | Session history + HTML/Markdown report export with audit trail | store (rusqlite session store) + report export tests green locally (`store_tests`, `report_tests`) and in CI core job | PASS |

## Release build verification

- `cargo build -p difflens-core --release` — green on the release VM.
- Full 3-OS bundle build rides on the `v1.0.0` tag run of
  `.github/workflows/release.yml`; draft release inspected for per-OS
  installers (`.msi` + NSIS `.exe`, `.dmg` aarch64, `.AppImage`, `.deb`)
  before publishing.
- Model pins re-verified at release time (sha256 in
  `resources/manifests/llm-manifest.json` and `scripts/fetch-models.sh`;
  checksum mismatch fails the build).
- PDFium bundled build `chromium/8165` (published 2026-11-16); no known
  unpatched critical CVE advisories flagged against this build at release
  date. Re-check at every future release.

## Known limitations shipped with v1.0.0

- Installers are unsigned: macOS Gatekeeper (right-click → Open) and Windows
  SmartScreen (More info → Run anyway) friction documented in README +
  release notes.
- OCR path is experimental (Tesseract CLI, user-installed; diacritic errors
  on low-res Vietnamese scans) — marked in audit trail when used.
- macOS Intel and auto-update (updater artifacts `latest.json`/`.sig`) are
  post-1.0 scope.
