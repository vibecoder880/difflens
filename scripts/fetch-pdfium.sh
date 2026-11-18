#!/usr/bin/env bash
# Fetch the pinned bblanchon/pdfium-binaries PDFium dynamic library used by
# difflens-core's PDF extractor (pdfium-render loads it at runtime).
#
# Idempotent: if the library is already present and the recorded checksum
# marker matches, this is a no-op. Re-run any time to force a refresh:
#   rm -rf resources/binaries/pdfium && scripts/fetch-pdfium.sh
#
# Extracts into resources/binaries/pdfium/ (gitignored), giving e.g.
#   resources/binaries/pdfium/lib/libpdfium.so        (Linux)
#   resources/binaries/pdfium/lib/libpdfium.dylib     (macOS)
#   resources/binaries/pdfium/bin/pdfium.dll          (Windows)
#
# Tests locate the library via DIFFLENS_PDFIUM_PATH (full file path); when
# unset they fall back to the path above relative to the repo root.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$REPO_ROOT/resources/binaries/pdfium"

# Pinned release (chromium build 8165, published 2026-11-16).
# URL-encode the "/" in the tag as %2F.
TAG="chromium/8165"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${TAG//\//%2F}"

# SHA256 checksums verified by downloading each archive from the pinned
# release on 2026-11-18 (sha256sum of the .tgz itself, not the contents).
checksum_for() {
  case "$1" in
    pdfium-linux-x64.tgz)    echo "ba7865e16c0271daab75256790be06c26272190afe6e2a70eae0bb39bb99089c" ;;
    pdfium-linux-arm64.tgz)  echo "338f5dcc907a20331812ae8ab01b16ff18966f4a247d790c8ad8fb49029d768b" ;;
    pdfium-mac-x64.tgz)      echo "07285915b564e46a432a9909dbc1f15892885ca6489e3faeefc70dee77ed371a" ;;
    pdfium-mac-arm64.tgz)    echo "112e74d717f3e0310273b70f3c8780e8a61ff11b34834124465c8305c841d350" ;;
    pdfium-win-x64.tgz)      echo "ca7338ae7779b8b1b82f4580305c4d762d214f8df1d54b47647fe3cc75989ded" ;;
    *) echo "" ;;
  esac
}

platform_asset() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux)
      # musl-based systems (Alpine) need the musl build.
      if (ldd --version 2>/dev/null || true) | grep -qi musl; then
        echo "UNVERIFIED-pdfium-linux-musl-x64.tgz"
      elif [ "$arch" = "x86_64" ]; then
        echo "pdfium-linux-x64.tgz"
      elif [ "$arch" = "aarch64" ]; then
        echo "pdfium-linux-arm64.tgz"
      else
        echo ""
      fi
      ;;
    Darwin)
      if [ "$arch" = "arm64" ]; then
        echo "pdfium-mac-arm64.tgz"
      else
        echo "pdfium-mac-x64.tgz"
      fi
      ;;
    MINGW* | MSYS* | CYGWIN*) echo "pdfium-win-x64.tgz" ;;
    *) echo "" ;;
  esac
}

ASSET="$(platform_asset)"
if [ -z "$ASSET" ]; then
  echo "fetch-pdfium: unsupported platform ($(uname -s)/$(uname -m)); skipping" >&2
  exit 0
fi

expected="$(checksum_for "$ASSET")"
if [ -z "$expected" ]; then
  echo "fetch-pdfium: no pinned checksum for $ASSET — refusing to download unverified archive" >&2
  exit 1
fi

marker="$DEST/.checksum"
library_rel=""
case "$ASSET" in
  pdfium-win-*) library_rel="bin/pdfium.dll" ;;
  *) library_rel="lib/libpdfium.so" ;;
esac

# Idempotency: already fetched this exact archive?
if [ -f "$DEST/$library_rel" ] && [ -f "$marker" ] && [ "$(cat "$marker")" = "$ASSET:$expected" ]; then
  echo "fetch-pdfium: already staged ($ASSET at $DEST) — nothing to do"
  exit 0
fi

echo "fetch-pdfium: downloading $BASE_URL/$ASSET"
TMPDIR_DL="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_DL"' EXIT

curl -fSL --retry 3 -o "$TMPDIR_DL/$ASSET" "$BASE_URL/$ASSET"

actual="$(sha256sum "$TMPDIR_DL/$ASSET" | cut -d' ' -f1)"
if [ "$actual" != "$expected" ]; then
  echo "fetch-pdfium: checksum mismatch for $ASSET" >&2
  echo "  expected: $expected" >&2
  echo "  actual:   $actual" >&2
  exit 1
fi
echo "fetch-pdfium: checksum ok ($actual)"

rm -rf "$DEST"
mkdir -p "$DEST"
tar xzf "$TMPDIR_DL/$ASSET" -C "$DEST" --no-same-owner
echo "$ASSET:$expected" > "$marker"
echo "fetch-pdfium: staged $DEST/$library_rel"
