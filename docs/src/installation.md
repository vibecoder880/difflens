# Installation

Installers are built by a public, SHA-pinned GitHub Actions workflow and published on the [releases page](https://github.com/vibecoder880/difflens/releases). No account, no license key, no server.

## Requirements

| | |
|---|---|
| OS | Windows 10/11 x64 · macOS 14+ Apple Silicon (no Intel build in v1) · Linux x64 |
| RAM | 8 GB recommended (Qwen3-4B explanations want ≥ 8 GB; the Qwen2.5-3B fallback fits 6 GB) |
| Disk | ~300 MB app + ~118 MB embedding model; optional AI models add 2.0–2.5 GB |

## Windows 10/11 x64

Download the `.msi` (or NSIS `.exe`). SmartScreen shows an unsigned-installer warning (v1 has no Authenticode certificate): click **More info → Run anyway**.

## macOS 14+ (Apple Silicon)

Open the `.dmg` (aarch64), drag DiffLens to Applications, launch. Gatekeeper blocks the first attempt because the build is unsigned — **right-click → Open**, then confirm. Needed once.

## Linux x64

```bash
# AppImage (needs FUSE: sudo apt-get install libfuse2)
chmod +x DiffLens_1.0.1_amd64.AppImage
./DiffLens_1.0.1_amd64.AppImage

# deb (system-wide)
sudo dpkg -i difflens_1.0.1_amd64.deb
```

## Why unsigned is safe here

The release workflow runs on GitHub-hosted runners from clean checkouts with third-party actions pinned to commit SHAs, and publishes SHA-256 checksums for every artifact.

## SHA-256 verification

Each release publishes checksums (`checksums.txt` / release notes).

**Linux / macOS:**

```bash
sha256sum -c checksums.txt            # one-shot
sha256sum DiffLens_1.0.1_amd64.AppImage   # manual compare
# macOS: use `shasum -a 256`
```

**Windows (PowerShell):**

```powershell
Get-FileHash .\DiffLens_1.0.1_x64-setup.exe -Algorithm SHA256
# compare against the published hash, or loop checksums.txt:
Get-Content checksums.txt | ForEach-Object {
  $expected, $name = ($_ -split '\s+', 2) | ForEach-Object { $_.Trim() }
  $actual = (Get-FileHash $name -Algorithm SHA256).Hash.ToLower()
  if ($actual -ne $expected.ToLower()) { "MISMATCH: $name" } else { "OK: $name" }
}
```

A mismatch means a corrupted or tampered download — re-download; if persistent, open an issue.

## After install

The installer is deliberately small. The in-app **Downloads** screen fetches what you need, each artifact SHA-256-verified against a compile-time-pinned manifest:

| Component | Size | Required | License |
|---|---|---|---|
| multilingual-e5-small (ONNX) | ~118 MB | yes (semantic alignment) | MIT |
| llama-server (llama.cpp b12503) | ~13–25 MB | for AI explanations | MIT |
| Qwen3-4B-Instruct-2507 GGUF | ~2.4 GB | optional (primary model) | Apache-2.0 |
| Qwen2.5-3B-Instruct GGUF | ~2.0 GB | optional (low-RAM fallback) | Apache-2.0 |

Details: [AI Explanations Setup](ai-setup.md).
