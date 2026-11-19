# Privacy and Security

Local-first **by architecture**: the code physically cannot send your documents anywhere, and CI enforces that invariant.

## Threat model

- Documents are sensitive; they must never leave the machine.
- Trust boundary: your own machine and user account. Compromised-webview hardening is defense-in-depth.
- The only network-adjacent surface is the Downloads manager — checksum-gated.

## Zero runtime egress

- No telemetry, crash reporting, analytics, or update phone-home.
- **`core/src/download/` is the only HTTP-capable module.** `scripts/audit-egress.sh` runs as the `egress-audit` CI job on every push; HTTP client code anywhere else fails the build.
- The LLM path talks to a **loopback** sidecar only; literal `127.0.0.1` / `[::1]` only (DNS names rejected — v1.0.1), re-validated per call; session keys (32 B OS randomness) travel via the `LLAMA_API_KEY` env var, never argv.

## Download integrity chain

1. **Pinned manifest** — URLs only in `resources/manifests/llm-manifest.json`, embedded at compile time; https + verified checksums mandatory (`verified: false` is fatal for primaries).
2. **HTTPS fetch** into `.part` with up-front (Content-Length) and streaming size caps.
3. **SHA-256 on the `.part` before the rename** — mismatch deletes the part; nothing installs.
4. **Atomic rename** into place; resume from `.part`; Cancel genuinely aborts.

## Archive extraction gates

`core/src/download/archive.rs` (adversarially tested): rejects absolute/`..`/prefix-escape paths; per-entry and total caps enforced while reading; tar links only relative + allowlisted + in-tree (zip symlinks always rejected); setuid/sticky stripped; staging wiped on failure. OOXML parsing adds zip-bomb budgets (`ZipBudget`), lying-header detection, XLSX row caps, 64 MB TXT cap, billion-laughs protection.

## Report escaping & path validation

- `html_escape` covers `& < > " '` **and** Unicode bidi overrides (U+202A–E, U+2066–9) in every context; reports contain zero JavaScript; Markdown fences and table cells escape correctly — all golden-tested.
- Backend validates webview-supplied paths: documents must be existing regular files; export targets must be absolute with a matching extension — no arbitrary-read/write primitives.
- OCR temp rasters are owner-only (0600) and deleted on drop.

## The review process

The v1.0.0 full code review ([`plans/20261117-difflens/reports/code-review-v1.0.0.md`](https://github.com/vibecoder880/difflens/blob/main/plans/20261117-difflens/reports/code-review-v1.0.0.md)) covered the entire security surface module-by-module:

- **P0: 0.** Checksum ordering, archive gates, manifest validation, loopback guard, HTML escaping verified correct and adversarially tested.
- **5 P1 + 10 P2 findings**, all fixed in v1.0.1: API key off argv, literal-only loopback, bounded 416 retry / 256 KiB streams / 50-job pruning, poisoning-proof locks, 0600 temp files, high-entropy keys, working download cancel, path validation, `spawn_blocking` pipeline, SHA-pinned Actions.
- Thirteen "verified-good" invariants documented for the future (parameterized SQL, strict CSP, zero `dangerouslySetInnerHTML`, subprocess hygiene, alignment determinism, …).

## What local-first does not mean

Not air-gapped (Downloads touches the network on your click) · not hardened against hostile same-user attackers · not a claim that LLM explanations are authoritative (see [How Diffing Works](how-it-works.md)).
