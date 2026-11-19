#!/usr/bin/env bash
# Fetch the pinned multilingual-e5-small ONNX model + tokenizer used by
# difflens-core's real embedder (OrtE5Embedder, feature `embeddings-ort`).
#
# Modes:
#   scripts/fetch-models.sh --test     # repo-root resources/models (CI/local tests)
#   scripts/fetch-models.sh --bundle   # packaged-app resources (src-tauri/resources/models)
#
# Idempotent: if the files are already present and match the recorded
# checksums, this is a no-op. Delete the directory to force a re-download:
#   rm -rf resources/models/e5-small && scripts/fetch-models.sh --test
#
# Extracts into resources/models/e5-small/ (gitignored), giving
#   resources/models/e5-small/onnx/model_quantized.onnx  (~118 MB, int8
#     quantized — deliberately NOT the same-size look-alikes
#     model_int8.onnx / model_uint8.onnx)
#   resources/models/e5-small/tokenizer.json
#
# Library code never downloads: this script is the only network egress for
# embeddings in the whole repository.
#
# Tests locate the model via DIFFLENS_E5_DIR (directory) or the default path
# above relative to the repo checkout.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --test: repo-root layout for CI/local tests; --bundle: packaged-app
# resources (src-tauri/resources/models) shipped via tauri.conf.json.
MODE="${1:---test}"
case "$MODE" in
  --test) DEST="$REPO_ROOT/resources/models/e5-small" ;;
  --bundle) DEST="$REPO_ROOT/src-tauri/resources/models/e5-small" ;;
  *)
    echo "usage: $0 [--test|--bundle]" >&2
    exit 2
    ;;
esac

# Pinned upstream export (Xenova/multilingual-e5-small, MIT-licensed base
# model intfloat/multilingual-e5-small). Commit resolved from the repo main
# branch on 2026-11-18; never re-resolve implicitly — bump explicitly.
PINNED_COMMIT="761b726dd34fb83930e26aab4e9ac3899aa1fa78"
BASE_URL="https://huggingface.co/Xenova/multilingual-e5-small/resolve/${PINNED_COMMIT}"

# SHA256 + exact byte sizes recorded from the pinned commit on 2026-11-18.
checksum_for() {
  case "$1" in
    onnx/model_quantized.onnx) echo "f80102d3f2a1229f387d3c81909990d8945513e347b0eab049f7de3c6f98c193" ;;
    tokenizer.json)            echo "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39" ;;
    *) echo "" ;;
  esac
}

size_for() {
  case "$1" in
    onnx/model_quantized.onnx) echo "118308185" ;;
    tokenizer.json)            echo "17082730" ;;
    *) echo "0" ;;
  esac
}

fetch_one() {
  local rel="$1"
  local dest="$DEST/$rel"
  local expected_sum expected_size actual_sum actual_size

  expected_sum="$(checksum_for "$rel")"
  expected_size="$(size_for "$rel")"

  # Present and correct → skip (idempotent).
  if [ -f "$dest" ]; then
    actual_sum="$(sha256sum "$dest" | cut -d' ' -f1)"
    if [ "$actual_sum" = "$expected_sum" ]; then
      echo "ok (cached): $rel"
      return 0
    fi
    echo "checksum mismatch on cached $rel — re-downloading" >&2
    rm -f "$dest"
  fi

  echo "downloading $rel from pinned commit ${PINNED_COMMIT} …"
  mkdir -p "$(dirname "$dest")"
  # -f: fail on HTTP errors (HF returns HTML error pages otherwise).
  # --http1.1: HTTP/2 stream resets (curl 92) are flaky on CI runners for
  #   large HF files — pin HTTP/1.1.
  # --retry-all-errors + -C -: retry any failure and resume the partial
  #   download instead of restarting from byte 0.
  curl -fL --http1.1 --retry 5 --retry-all-errors --retry-delay 2 -C - \
    -o "$dest" "$BASE_URL/$rel"

  actual_sum="$(sha256sum "$dest" | cut -d' ' -f1)"
  if [ "$actual_sum" != "$expected_sum" ]; then
    echo "FATAL: sha256 mismatch for $rel" >&2
    echo "  expected $expected_sum" >&2
    echo "  actual   $actual_sum" >&2
    exit 1
  fi
  actual_size="$(wc -c < "$dest" | tr -d ' ')"
  if [ "$actual_size" != "$expected_size" ]; then
    echo "FATAL: byte size mismatch for $rel (expected $expected_size, got $actual_size)" >&2
    exit 1
  fi
  echo "ok: $rel ($actual_size bytes, sha256 verified)"
}

# tokenizer.json missing is a hard failure (presence re-verified by size +
# checksum below anyway); this guard documents the contract loudly.
if [ -f "$DEST/tokenizer.json" ] && [ ! -s "$DEST/tokenizer.json" ]; then
  echo "FATAL: tokenizer.json exists but is empty — delete $DEST and re-run" >&2
  exit 1
fi

fetch_one "tokenizer.json"
fetch_one "onnx/model_quantized.onnx"

echo "e5-small model ready at $DEST"
