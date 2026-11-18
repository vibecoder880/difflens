# DiffLens v1.0.0 — Full Code Review

- Date: 2026-11-18 · HEAD `5d08394` (tag `v1.0.0`) · Read-only review, no files modified.
- Scope: full security surface (download/, llm/, report/, extract/, src-tauri/), correctness (align/, diff/, heuristic/, store/, ocr/), frontend Tauri boundary, CI/build, cross-cutting determinism. Source was read module-by-module and traced end-to-end (file → extract → align → diff → report/UI).
- Tooling run: `cargo clippy -p difflens-core --all-targets` (clean), `npx tsc --noEmit` (clean), `npm test` (47 vitest tests pass). Core has 287 `#[test]` fns (234 unit + 53 integration).

## Overall Assessment

High-quality, unusually well-hardened codebase for a v1.0.0 local-first desktop app. The security-critical paths (download checksum gating, archive extraction gates, loopback guard, HTML escaping) are implemented correctly, documented with their threat model, and covered by adversarial tests. Zero P0 findings. Five P1 findings — one functional bug (download cancel is dead-wired), one robustness bug (unbounded 416 retry loop), one defense-in-depth gap (webview → arbitrary paths), one CI supply-chain hygiene item, one least-privilege cleanup.

---

## P0 — Must fix before wider adoption

**None found.** The checksum-before-rename ordering, archive path/link gates, manifest validation, loopback guard, and HTML escaping — the paths where a P0 could hide — are all correct and adversarially tested.

---

## P1 — Should fix

### P1-1. Download cancellation is dead-wired; `DownloadManager` is never used
- **Where:** `/home/khanh/Documents/difflens/src-tauri/src/commands.rs:587-601` (server), `:672-687` (model), `:853-866` (`llm_cancel`); UI: `/home/khanh/Documents/difflens/src/routes/Downloads.tsx:93`.
- **Evidence:** Both `llm_install_server` and `llm_download_model` call `download_file` directly on bare threads, and their progress closures unconditionally return `true`:
  ```rust
  let result = difflens_core::download::download_file(&spec, &mut |done, total| {
      ... emit(...);
      true            // <-- never reads a cancel flag
  })
  ```
  `llm_cancel` only consults `state.llm.downloads.cancel(&job_id)` (`DownloadManager`), but no job is ever *started* through that manager (`grep downloads src-tauri/src` shows only the two cancel lookups). Job ids issued by these commands are `server-<tag>` / `model-<id>`, not `dl-N`.
- **Impact:** The Cancel button shown during a 2.5 GB model / ~65 MB server download does nothing; the download runs to completion (or failure) and the UI never leaves the running state. `state.llm.downloads` is effectively dead code that implies a capability the app does not have.
- **Fix:** Route both downloads through `DownloadManager::start` (it already handles progress, cancel flags, and terminal snapshots), emit from the manager's `on_progress`, and keep `llm_cancel` as-is. Alternatively, pass an `Arc<AtomicBool>` into the progress closures (`!flag.load(...)`).

### P1-2. Unbounded retry loop on persistent HTTP 416
- **Where:** `/home/khanh/Documents/difflens/core/src/download/mod.rs:148-154`.
- **Evidence:**
  ```rust
  (_, 416) => {
      let _ = std::fs::remove_file(&part);
      start_at = 0;
      continue;        // matches even when start_at == 0
  }
  ```
  The `(_, 416)` arm fires regardless of `start_at`. After the first 416 resets `start_at = 0`, a server that (wrongly or maliciously) answers the subsequent plain GET with 416 again sends the loop back here forever. There is no attempt counter.
- **Impact:** The download thread spins indefinitely issuing network requests; the job stays `running` forever and the app must be restarted. Bounded by network round-trips, so no memory growth — but it is an unbounded loop in the sole network-egress path.
- **Fix:** Track a `restarted: bool`; only take the 416 branch when `start_at > 0`, otherwise return an error ("server returned 416 for a full GET").

### P1-3. Webview-facing commands accept arbitrary filesystem paths (defense-in-depth)
- **Where:** `/home/khanh/Documents/difflens/src-tauri/src/commands.rs:38-78` (`run_diff`: any `old_path`/`new_path` string), `:233-271` (`export_report`: any `target_path`).
- **Evidence:** `run_diff` invokes `std::fs::read(path)` on whatever the webview passes; the extracted content lands in the job result, the history DB, and the LLM prompt. `export_report` writes rendered bytes to any path that passes only `is_dir()` / parent-exists checks.
- **Impact:** Threat model is a local app with trusted UI, so this is not directly exploitable today (CSP is strict, zero `dangerouslySetInnerHTML`, no remote content). But if the webview is ever compromised, these two commands are a ready-made arbitrary-file-read primitive (any file the user can read → rendered into the UI/history, a place an attacker can then read) and arbitrary-file-write. Given the release is open-source and gains contributors, tighten now.
- **Fix:** Minimum: document the trust boundary in `commands.rs` and keep file *picking* exclusively behind the dialog plugin (the frontend already does this — enforce it backend-side by rejecting paths outside a recent-dialog result, or at least reject dotfiles/system dirs). For `export_report`, the save-dialog provenance is already the design; consider echoing the dialog-issued path through a short-lived token rather than trusting the caller string.

### P1-4. Release workflow: third-party actions pinned to mutable refs in a `contents: write` job
- **Where:** `/home/khanh/Documents/difflens/.github/workflows/release.yml` (`jlumbroso/free-disk-space@main`, `tauri-apps/tauri-action@v0`), plus `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `actions/setup-node@v4`.
- **Impact:** This job holds `contents: write` and `TAURI_SIGNING_PRIVATE_KEY`. A compromised or hijacked upstream repo (the `@main` ref in particular) executes with those credentials and publishes installers. `@main` is the worst-case pin; `@v0` floats across all v0.x.
- **Fix:** Pin every third-party action to a full commit SHA (dependabot/renovate can maintain them). Priority on `free-disk-space@main` and `tauri-action@v0`.

### P1-5. Unused, over-broad capability grants (`shell:allow-spawn`, `shell:allow-stdin-write`)
- **Where:** `/home/khanh/Documents/difflens/src-tauri/capabilities/default.json:6-10`; `/home/khanh/Documents/difflens/src-tauri/src/lib.rs:19` registers `tauri_plugin_shell`; `/home/khanh/Documents/difflens/package.json` ships `@tauri-apps/plugin-shell` — none of them are used (`grep` for `plugin-shell` / `Command.` in `src/` finds no frontend usage; llama-server and tesseract are spawned from Rust, which needs no capability).
- **Impact:** Today fail-closed: the shell plugin denies spawn with no scope entries, so there is no live escalation. But the grant is pure attack surface with zero payoff — any future scope misconfiguration or plugin behavior change turns an unused grant into a webview→process bridge.
- **Fix:** Delete both permissions (and the plugin registration + npm dep) or scope them to nothing. The Rust-side spawns do not require them.

---

## P2 — Nice-to-have

### P2-1. Bearer API key passed on the llama-server command line
- `core/src/llm/sidecar.rs:231-246` — `--api-key <key>` is visible in the process table (`ps`) to any same-user process. Threat model is same-user, so no boundary is crossed, but a config file/stdin hand-off would remove the exposure. Documented "not a password" trade-off is honest; keep it documented.

### P2-2. `validate_loopback` accepts the DNS name `localhost`
- `core/src/llm/client.rs:227-262` — `localhost` resolution depends on the resolver; a hostile resolver could point it elsewhere while the client attaches `Authorization: Bearer <session key>`. Unreachable through the app itself (the sidecar always builds `http://127.0.0.1:<port>`, sidecar.rs:291), only via manual `LlmClient::new`. Consider rejecting DNS names and keeping only literal `127.0.0.1` / `[::1]`, or resolving and checking the addresses.

### P2-3. ureq follows redirects to arbitrary hosts
- `core/src/download/mod.rs:124-129` — default ureq redirect policy (10 hops, cross-host). Mitigated: URLs come only from the compile-time-pinned manifest and the SHA-256 gate runs before the atomic rename, so a redirecting mirror cannot ship different bytes. No credentials are sent on downloads. Note it in the module docs; optionally restrict redirects or disable them (`ureq` cannot currently — verify per-release behavior).

### P2-4. `SidecarManager::ensure_running` holds its lock across the 60-second spawn
- `core/src/llm/sidecar.rs:401-422` — `active_info()` (used by `llm_status`, polled from the UI) shares the same mutex, so status queries block until spawn/health-poll finishes. Not a deadlock, but a UI-latency hazard. Clone the needed info under a short lock or use a parking-lot-free design (spawn outside the lock, then swap).

### P2-5. Diff job map grows without bound
- `src-tauri/src/state.rs:150-247` — `JobRecord`s (including full `DiffResult` blobs) are never removed; a long session accumulates every result in memory. Add pruning (keep last N, or drop results for jobs older than X once persisted to the store).

### P2-6. OCR temp TIFFs are never cleaned up, in a shared predictable directory
- `core/src/ocr/engine.rs:469-483` — writes document-derived TIFFs to `%TEMP%/difflens-ocr/page-<pid>-<nanos>.tiff` with default permissions and deletes nothing. Name entropy (pid + ns timestamp) is adequate against prediction, but on multi-user hosts the file is readable by others until reboot. Delete in a guard (`Drop`) and/or use `tempfile::NamedTempFile` with mode 0600.

### P2-7. SSE accumulated text is unbounded within a request
- `core/src/llm/client.rs:182-219` — `text` grows with every token; the only bounds are `max_tokens: 512` (client-chosen) and the 300 s overall timeout, plus the 1 MiB carry guard at `:312`. A misbehaving server that ignores `max_tokens` can push large memory. Cap `text.len()` (e.g. 256 KiB) and error out.

### P2-8. Pipeline runs blocking on the Tauri async runtime
- `src-tauri/src/commands.rs:54` — `tauri::async_runtime::spawn` wraps the fully synchronous, CPU-heavy `run_with_progress`. This occupies a runtime worker per job (progress emits and store writes happen inline). Fine today (one job at a time in practice), but use `spawn_blocking` semantics to avoid starving the runtime if concurrency is ever added.

### P2-9. Download manager panic paths on lock poisoning
- `core/src/download/mod.rs:323, 337-342, 347-350` — `.expect("download job map poisoned")` / `.expect("job record present")` inside download threads. If any thread ever panics while holding the jobs mutex, every later `snapshot()`/`cancel()` call from the Tauri command thread panics too. Same pattern in `state.rs` (`expect("job map lock poisoned")`). Local threat model makes triggering this unlikely; still, prefer `lock().map_err(...)` at the Tauri boundary (the store handle already does this correctly at commands.rs:150-155).

### P2-10. `random_api_key` entropy is modest
- `core/src/llm/sidecar.rs:171-185` — time + pid + ASLR address + counter, hashed. Adequate for a loopback-only session key (documented), but `getrandom`/`rand` would remove the argument entirely. Also note the counter makes keys predictable in isolation if time/pid/ASLR are known.

---

## Verified-good (do not regress these when fixing)

1. **Checksum ordering is correct.** `download_file` verifies SHA-256 on the `.part` *before* the rename into `dest` (`core/src/download/mod.rs:217-227`), deletes the part on mismatch, enforces size caps both up-front (Content-Length) and while streaming, and validates `expected_bytes` from the manifest. Manifest itself is embedded at compile time (`manifest.rs:22-23`), requires https + provenance + exactly one primary, and rejects unverified primaries/assets — a tampered shipped manifest cannot disable checksums (`verified: false` is fatal for server assets and primaries, `manifest.rs:215-219, 245-250`).
2. **Archive extraction gates are thorough.** `gate_path` rejects absolute/`..`/prefix components; per-entry and total caps enforced *while reading*, not from headers; tar symlinks/hardlinks allowed only with relative, allowlisted, in-tree targets; zip symlinks always rejected; setuid/sticky stripped; staging dir wiped on any failure and must be empty to start. Adversarial tests cover raw-header traversal, absolute paths, link escapes, oversized entries, disallowed basenames (`archive.rs:528-933`).
3. **OOXML zip-bomb budgets.** `ZipBudget` + `read_zip_entry_limited` (`core/src/extract/mod.rs:124-183`) reserve declared size, cap per-part bytes, and detect lying headers via a one-byte-past `take`. roxmltree 0.21.2 keeps default entity-expansion limits (billion-laughs protected). XLSX row cap with truncation warning; TXT 64 MB cap.
4. **Loopback guard** rejects `0.0.0.0`, other 127.x, DNS names, missing schemes; re-validated on every call (`client.rs:146`). Port parsing trusts only literal `127.0.0.1` bind lines (`sidecar.rs:162`). Kill-on-Drop + reap for the sidecar, kill + wait on every failure path; stale-server replacement is handled.
5. **SSE parser robustness.** Handles split chunks/mid-UTF-8 frames, skips malformed lines without panicking, surfaces server error frames, and caps unbounded carry at 1 MiB (`client.rs:311-314`). API key lives in the struct only and is never logged (grep-clean).
6. **HTML report escaping is complete.** `html_escape` covers `& < > " '` and Unicode bidi override chars (U+202A–E, U+2066–9); every document-derived string — paths, spans, flag values, details — passes through it, including attribute contexts (`id="{...}"` at html.rs:204). Zero JS in the report. Markdown fences grow past the longest backtick run; table cells escape pipes/newlines (`markdown.rs:200-224, 285-287`). Golden hostile-content tests enforce both.
7. **Alignment determinism.** NW full-matrix with threshold-gated diagonals, exact-float recomputation traceback with fixed preference order (diag > up > left), chunk windows locked in lockstep; anchors resolved one-to-one via sorted candidates (HashMap iteration order irrelevant — `candidates.sort()` before use); move-repair tie-breaks on (sim, dist, index) with epsilon guards; final pairs sorted. No wall-clock/randomness in core except OCR timing metadata and the deliberate API-key nonce.
8. **Delta math.** All three relative-delta divisions guard `|a| > 1e-9` (`deltas.rs:155-158, 202-205, 217-220`); pairing keys are quantized to `i64` for total ordering; references pair by normalized equality then positionally. The ambiguous `1.500` (dot-thousands vs decimal) case is handled and tested — bare `1.500.000` without a currency marker is rejected as money (`entities.rs:855`).
9. **Severity determinism.** Lexicon categories collected then `sort_unstable()`ed (`severity.rs:173`); score = max rule points + keyword uplift; bands fixed constants.
10. **Store.** All queries parameterized (`store/mod.rs` — no string interpolation into SQL anywhere); WAL + busy timeout; corrupt JSON degrades instead of failing; 64 MB blob cap with warning; explicit `created_at` injection keeps core wall-clock-free.
11. **OCR subprocess hygiene.** `run_with_timeout` drains pipes on threads (no deadlock), kills + reaps on timeout, survives grandchildren holding pipes, bounded grace period.
12. **Frontend boundary.** `src/lib/tauri.ts` is fully typed with a single `invoke` choke point and graceful browser-dev rejection; **zero** `dangerouslySetInnerHTML` in the repo; strict CSP (`default-src 'self'`, no unsafe-eval); event listeners unregistered with a cancelled-flag guard on unmount (`Diff.tsx:42-59`, `Home.tsx:50-81`); download history persists via localStorage with terminal-state filtering; i18n interpolation uses ICU-style `{{var}}` with an EN/VI parity test suite.
13. **CI.** Bounded 3-attempt retries that `exit 1` after exhaustion (they cannot mask persistent failures); "assert tests actually ran" greps (`pdf-extraction-ok`, `ocr-ci-ok`) defeat silent skipping; egress-audit job enforces the HTTP-only-in-download/ invariant; embeddings-ort feature is compiled and smoke-tested on its own job.

## Metrics
- Clippy (core, all targets): clean (manifest-level warnings only). tsc: clean. Vitest: 47/47 pass.
- Core tests: 287 `#[test]` fns; adversarial red-team suites present for download/report.
- Linting issues: 0 code; 3 workspace-manifest warnings (cosmetic).
- P0: 0 · P1: 5 · P2: 10.

## Verdict
**0 ship-blocking findings.** Ship v1.0.0. Recommended immediately post-release: P1-1 (broken download cancel — user-visible bug), P1-2 (416 loop), P1-5 (capability trim), P1-4 (action pinning); P1-3 belongs in the first hardening pass.
