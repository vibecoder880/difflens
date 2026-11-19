# AI Explanations Setup

Optional: explains a single change in natural language (EN or VI) with a small LLM running entirely on your machine. Everything else works without it.

## How it works

DiffLens downloads a pinned **`llama-server`** (llama.cpp release `b12503`, MIT) and spawns it on demand:

- OpenAI-compatible `/v1/chat/completions` with SSE streaming; 2048-token context, output capped at 512 tokens.
- Binds **loopback only** (`127.0.0.1`, ephemeral port) with a per-session key.
- Fully managed lifecycle: spawn → health-poll → kill on exit; stale servers are replaced.

## What gets downloaded, where

Under the OS app-data directory:

```
<app_data_dir>/
├── bin/llama-server-b12503/          # sidecar bundle (server + shared libs)
├── models/*.gguf                     # explanation models
└── difflens.sqlite3                  # history (unrelated)
```

Windows: `%APPDATA%\app.difflens.desktop` · macOS: `~/Library/Application Support/app.difflens.desktop` · Linux: `~/.local/share/app.difflens.desktop`.

## Models

| Model | Size | Role | Min RAM | License |
|---|---|---|---|---|
| Qwen3-4B-Instruct-2507 (Q4_K_M) | ~2.4 GB | primary; strong Vietnamese | 8 GB | Apache-2.0 |
| Qwen2.5-3B-Instruct (Q4_K_M) | ~2.0 GB | fallback for 6–8 GB machines | 6 GB | Apache-2.0 |
| Qwen3-0.6B (Q8_0) | ~0.6 GB | CI smoke model | — | Apache-2.0 |

The first manifest entry present on disk is used (primary → fallback → test).

## Download integrity

- URLs come **only** from the compile-time-embedded manifest (`resources/manifests/llm-manifest.json`); https required, `verified: false` entries are fatal.
- Download streams to a `.part` file (size caps checked up-front and while streaming), **SHA-256 verified before the atomic rename**; mismatch deletes the part.
- Resume supported from `.part`; Cancel genuinely aborts while keeping the part.

## Privacy properties

1. **Loopback-only**: literal `127.0.0.1` / `[::1]` accepted — DNS names (even `localhost`) rejected since v1.0.1, so a hostile resolver can't steal the bearer token; re-validated per call.
2. **Key off argv**: the 32-byte (OS randomness) session key is passed via the `LLAMA_API_KEY` environment variable, invisible to the process table.
3. **Sole-egress principle**: `core/src/download/` is the only module allowed to perform HTTP — enforced by the `egress-audit` CI job running `scripts/audit-egress.sh` on every push; the LLM client is additionally loopback-restricted.
4. **No telemetry**: no crash reporting, no analytics, no phone-home.

## Planning

Disk: ~2.5 GB + ~65 MB + ~118 MB headroom for parts. RAM: pick the 3B fallback under 8 GB. First explanation includes model-load time; later ones reuse the running sidecar.

## Troubleshooting

| Symptom | Fix |
|---|---|
| "no model downloaded yet" | Fetch a model on the Downloads screen |
| "llama-server not installed" | Install the sidecar (or have one on `PATH`) |
| Model load failure / OOM | Use the Qwen2.5-3B fallback; the SHA-256 gate refuses corrupted files |
| Stuck download | Cancel + resume (real abort since v1.0.1; `.part` kept) |
| Stream stopped | Cancelled or hit the 256 KiB output cap — request again |

Events: `llm://download` (installs), `llm://token` (deltas), `llm://done` (terminal summary/impact or error).
