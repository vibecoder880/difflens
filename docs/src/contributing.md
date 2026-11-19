# Contributing

Thanks for helping build a local-first document differ. The full rules live in [CONTRIBUTING.md](https://github.com/vibecoder880/difflens/blob/main/CONTRIBUTING.md) in the repository — the short version:

- **Gates before merge**: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test -p difflens-core`, `npm test`, `npm run build`, `scripts/audit-egress.sh` — CI runs the same gates plus the ignored tiers (PDF, real embeddings, OCR, LLM roundtrip).
- **The egress rule**: any new network egress must live in `core/src/download/`, use manifest-pinned URLs, verify SHA-256, and pass `scripts/audit-egress.sh`. The LLM client may only talk to loopback. No telemetry, no analytics.
- **Commits**: Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`).
- **i18n**: keep UI strings in both `src/i18n/en.json` and `src/i18n/vi.json`; the parity test fails otherwise.
- **Determinism**: severity classification stays rule-based; LLM code paths must never alter diff results or severity bands.
- **Pins**: bump versions deliberately; update `THIRD-PARTY-NOTICES.md`.

See [Building from Source](building.md) for the toolchain and test tiers.
