---
title: "Phase 8: Persistence + reports (rusqlite history, HTML/Markdown export)"
status: todo
priority: P2
effort: 24h
---

# Phase 8: Persistence + Reports

## Context Links

- Plan: [plan.md](./plan.md)
- Phases 2–4 output types (DiffResult, AuditTrail); Phase 7 history/settings stubs to replace.

## Overview

**Priority:** P2 · **Status:** pending · **Effort:** ~24h
Persist diff sessions in rusqlite (bundled SQLite in `app_data_dir/difflens.db`) with file metadata, options snapshots and full results; build the History page with re-run and delete; implement HTML and Markdown report exports (standalone, audit trail included) saved via the dialog plugin. Reports localize labels per the session's language (EN/VI).

## Key Insights

- rusqlite `bundled` feature compiles SQLite in — no system dependency drift across the 3 CI OSes.
- Store the serialized `DiffResult` JSON blob plus a small severity summary row for fast history listing; re-run re-extracts from original paths when they still exist, else prompts re-pick (document hashes make staleness visible).
- Reports are pure functions of the stored session — golden tests lock HTML structure and Markdown table shape; no external assets in the HTML (inline CSS only) so it opens from disk anywhere.

## Requirements

- [ ] SQLite store with migration v1: sessions table (metadata, options JSON, result JSON, severity summary).
- [ ] Commands: save on diff completion, list sessions, get session, delete session, re-run.
- [ ] Re-run with staleness detection (file hash vs stored hash; missing-file prompt).
- [ ] HTML report (inline CSS, side-by-side table, change list, severity, audit trail, EN/VI labels) + Markdown report.
- [ ] Export via save dialog to user-chosen path.
- [ ] All document-derived strings HTML-escaped via a single shared helper in `report/html.rs`; hostile-content (XSS payload) golden test green.
- [ ] `cargo test -p difflens-core -- store report` green; Phase 7 history/settings stubs replaced.

## Architecture

`core/src/store/mod.rs` owns `Store(SqliteConnection)` opened at app start (state); `schema.rs` holds migration v1 DDL; all queries parameterized. `core/src/report/mod.rs` renders from a stored session: `report/html.rs` (string-template rendering with inline CSS, no deps) and `report/markdown.rs`. Tauri commands in `src-tauri/src/commands.rs` thin-wrap core calls; export uses `tauri-plugin-dialog` save dialog then writes the file.

## Related Code Files

- `core/src/store/mod.rs` (create — Store type, queries)
- `core/src/store/schema.rs` (create — migrations v1)
- `core/src/report/mod.rs`, `core/src/report/html.rs`, `core/src/report/markdown.rs` (create)
- `core/tests/store_tests.rs`, `core/tests/report_tests.rs` (create)
- `src-tauri/src/commands.rs` (modify — add `list_sessions`, `get_session`, `delete_session`, `rerun_session`, `export_report`)
- `src-tauri/src/state.rs` (modify — add Store to the Tauri State)
- `src/routes/history.tsx`, `src/components/history/SessionCard.tsx` (modify/create — replace stubs)
- `src/lib/tauri.ts`, `src/lib/types.ts` (modify — add the session/export command types and the `listSessions`/`deleteSession`/`exportReport` wrappers deferred from Phase 7)

## Implementation Steps

1. `schema.rs`: migration v1 — `sessions(id TEXT PK, created_at INTEGER, lang TEXT, file_a_name TEXT, file_a_hash TEXT, file_a_size INTEGER, file_b_name TEXT, file_b_hash TEXT, file_b_size INTEGER, options_json TEXT, result_json TEXT, severity_summary_json TEXT, change_count INTEGER)`; `migrations` table bookkeeping; id = ULID/uuid v7 string.
2. `store/mod.rs`: `save_session`, `list_sessions` (paged, newest first, summary only), `get_session`, `delete_session`; parameterized SQL only; DB path under `app_data_dir` (fs plugin dir resolution on Rust side).
3. Wire save: `run_diff` completion persists the session (the Phase 3 command, modified here); `get_diff` falls back to store for past sessions.
4. Re-run: `rerun_session(session_id)` re-reads stored file paths, recomputes SHA256, compares to stored hashes — mismatch → `StaleInput` error with stored/actual hash info so the UI can prompt re-pick; missing file → `FileMissing` error.
5. `report/html.rs`: standalone document — inline CSS, header (files, dates, options), severity summary chips, side-by-side pair table with inline spans, change list with audit-trail appendix; a single `html_escape()` helper is applied to **all** document-derived strings (block text, excerpts, file names, audit spans) — no raw document text ever enters the HTML unescaped (document text is attacker-controlled; this is the stored-XSS-in-export defense); label strings resolved from EN/VI bundles on the Rust side (small label map mirroring i18n keys; parity-tested).
6. `report/markdown.rs`: headings, metadata table, change table (id, op, severity, excerpt), audit appendix; escape pipes/HTML.
7. Export commands: `export_report(session_id, format)` → dialog save → write bytes → return path; UI adds export buttons on result page + history card ("Open in folder" via opener plugin).
8. Phase 7 integration: replace history stub route (list, filter by severity, open result, re-run with staleness dialog, delete with confirm); settings "clear history" now calls `delete_session` for all; add the `listSessions`/`deleteSession`/`exportReport` wrappers to `src/lib/tauri.ts` (deferred from Phase 7).
9. Tests: store roundtrip (temp-dir DB, save/list/get/delete), migration test (v1 fresh + reopen), re-run staleness tests, HTML golden test (contains change markers + audit rows + inline CSS only), hostile-content golden test — a DOCX fixture containing `<script>`/`onerror=` payloads is asserted escaped in both the HTML and Markdown output, Markdown golden test (valid tables), label parity test EN/VI.

## Todo

- [ ] Schema v1 + Store queries with tests
- [ ] Save-on-complete + get_diff fallback wiring
- [ ] Re-run with staleness/missing-file handling
- [ ] HTML + Markdown report renderers with golden tests
- [ ] Export via dialog + open-in-folder
- [ ] History page + clear-history settings wired
- [ ] HTML-escape helper on all document-derived strings + hostile-content golden test
- [ ] `cargo test -p difflens-core -- store report` green

## Success Criteria

- `cargo test -p difflens-core -- store report` exits 0 (temp-dir DBs; no user dirs touched).
- Hostile-content golden test passes: `<script>`/event-handler payloads from a DOCX fixture are escaped in both HTML and Markdown output (no raw injection survives export).
- `cargo test --workspace` still exits 0 (Phase 7 stubs replaced without regressions).
- Manual: history lists past sessions; re-run of an unchanged session reproduces byte-identical result JSON (determinism carried from Phase 3/4); exported HTML opens from disk with full styling and audit trail; Markdown renders in a standard renderer.

## Risk Assessment

- Result JSON blobs grow with document size → store summary row for listing; full JSON loaded only on open; document a size guard (skip storing >64 MB results with warning).
- rusqlite bundled build time on CI → acceptable one-time compile cost, cached via registry cache only.
- Report HTML regression → golden tests + structure asserts.

## Security Considerations

- Parameterized SQL exclusively (no string-built queries); DB file under app data dir with user-scoped permissions.
- HTML reports escape every document-derived string (block text, excerpts, file names, audit spans) through one shared `html_escape()` helper — DOCX/PDF/PPTX text is attacker-controlled and reports are opened from disk in browsers, so unescaped injection would be stored-XSS-in-export; the hostile-content golden test enforces it.
- Export writes only to the user-selected path via dialog; no telemetry, no network; LLM output is never included in reports (audit trail only).

## Next Steps

Phase 9 finalizes icons/metadata, verifies the signed 3-OS release pipeline end-to-end, adds README/screenshots/attribution, runs the E2E smoke, and tags v1.0.0.
