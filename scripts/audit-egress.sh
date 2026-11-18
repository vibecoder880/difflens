#!/usr/bin/env bash
# DiffLens egress audit (phase-05 acceptance g).
#
# Static check that HTTP-client constructions and URL literals appear
# ONLY inside the allowlisted modules — the download manager (the app's
# single network egress point) and the loopback-only LLM client.
#
# Scope A (shipped code): core/src, src-tauri/src, src (frontend)
#
# Two tiers, each with its own allowlist:
#   Tier 1 — HTTP client construction:
#     pattern:   ureq:: | reqwest:: | \bClient::new
#     allowlist: core/src/download/*  (incl. the shared ureq agent builder)
#                core/src/llm/client.rs (loopback-guarded SSE client;
#                contract-sanctioned egress point)
#   Tier 2 — URL literals / frontend fetch:
#     pattern:   https?:// | fetch(
#     allowlist: same as tier 1, plus two documented exclusions:
#       (a) OOXML namespace identifiers (schemas.openxmlformats.org) —
#           XML vocabulary in core/src/extract/*, never dialed;
#       (b) URL literals that also mention 127.0.0.1 / localhost —
#           loopback-only by construction; the client additionally
#           validates every base URL at construction (unit-tested).
#
# KNOWN LIMITATION / false-positive risk (documented, accepted):
# - This is a grep audit, not a dataflow audit: a determined violator
#   could obfuscate a URL (string concatenation, base64, runtime-built
#   hostnames). It reliably catches accidental egress — a new `ureq`
#   call, a copied remote URL constant, a frontend `fetch()` — which is
#   the failure mode that matters here.
# - Test code is NOT scanned: deterministic tests spin up 127.0.0.1
#   loopback mock servers and the loopback-guard tests intentionally
#   embed hostile remote URL literals to assert rejection. The ignored
#   real-network test downloads exclusively through the audited manager.
# - `fetch(` matches non-network helpers in theory; none exist today —
#   a hit means review it.

set -euo pipefail
cd "$(dirname "$0")/.."

tier1='ureq::|reqwest::|\bClient::new'
tier2='https?://|fetch\('

report="$(mktemp)"
trap 'rm -f "$report"' EXIT

fails=0

# ---- Tier 1: raw HTTP client construction ----
grep -rnE "$tier1" \
  --include='*.rs' --include='*.ts' --include='*.tsx' \
  core/src src-tauri/src src \
  2>/dev/null \
  | grep -v -e '^core/src/download/' -e '^core/src/llm/client\.rs:' \
  > "$report" || true
if [ -s "$report" ]; then
  fails=1
  echo "TIER 1 FAILED — raw HTTP client construction outside the allowlist:"
  cat "$report"
  echo
fi

# ---- Tier 2: URL literals / frontend fetch ----
grep -rnE "$tier2" \
  --include='*.rs' --include='*.ts' --include='*.tsx' \
  core/src src-tauri/src src \
  2>/dev/null \
  | grep -v -e '^core/src/download/' -e '^core/src/llm/client\.rs:' \
  | grep -v 'schemas\.openxmlformats\.org' \
  | grep -v -e '127\.0\.0\.1' -e 'localhost' \
  > "$report" || true
if [ -s "$report" ]; then
  fails=1
  echo "TIER 2 FAILED — URL literals / fetch() outside the allowlist:"
  cat "$report"
  echo
fi

if [ "$fails" -ne 0 ]; then
  echo "Allowed locations: core/src/download/*, core/src/llm/client.rs"
  echo "Documented exclusions: OOXML namespace constants; 127.0.0.1/localhost literals."
  echo "If this is a legitimate new egress point, extend the allowlist in"
  echo "scripts/audit-egress.sh deliberately, never accidentally."
  exit 1
fi

# Sanity: the allowlist itself must still contain the egress (guards
# against the audit passing because the code was moved away, not because
# it is clean).
if ! grep -rqE "$tier1" core/src/download core/src/llm/client.rs; then
  echo "EGRESS AUDIT WARNING: allowlist is empty — expected HTTP usage in"
  echo "core/src/download/ and core/src/llm/client.rs. Audit may be broken."
  exit 1
fi

echo "egress audit ok: HTTP usage confined to core/src/download/ + core/src/llm/client.rs"
