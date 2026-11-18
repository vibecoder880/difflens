# Contributing to DiffLens

Thanks for helping build a local-first document differ! Short rules to keep
reviews fast and the release pipeline green.

## Build setup

```bash
# Rust stable via rustup (https://rustup.rs)
# Node.js 24 + npm
npm ci

# Linux only: Tauri system libraries
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config

npm run tauri dev      # run the app
```

## Gates (must pass before merge)

```bash
cargo fmt --check -p difflens-core
cargo clippy -p difflens-core --all-targets -- -D warnings
cargo test -p difflens-core
npm test          # vitest + EN/VI i18n parity vs backend label keys
npm run build
scripts/audit-egress.sh
```

CI runs the same gates on every push, plus the ignored tiers (PDF via pinned
PDFium, real e5 embeddings, OCR, LLM roundtrip).

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org):
`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:` — CI release
changelogs and review tooling rely on the format.

## The egress rule

**Any new network egress must live in `core/src/download/`** and pass
`scripts/audit-egress.sh`. Downloads must use URLs pinned in
`resources/manifests/llm-manifest.json` (or a script-managed pin) and verify
SHA-256 checksums. The LLM client may only talk to loopback. No telemetry,
no analytics — this is enforced by a grep gate in CI and PRs that break it
will not merge.

## Other notes

- Version pins live in `Cargo.toml` / `package.json` / the manifests; bump
  them deliberately and update `THIRD-PARTY-NOTICES.md`.
- Severity classification is deterministic by design: LLM code paths must
  never alter diff results or severity bands.
- Keep UI strings in both `src/i18n/en.json` and `src/i18n/vi.json` — the
  parity test fails otherwise.
