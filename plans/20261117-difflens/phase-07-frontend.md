---
title: "Phase 7: Frontend app (wizard, side-by-side view, change list, i18n, settings)"
status: todo
priority: P1
effort: 48h
---

# Phase 7: Frontend Application

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.9 (React 19 + Vite 7 + Tailwind 4 + shadcn/ui Radix + TanStack Query/Router + i18next 27); Phases 2–6 command/event surfaces.

## Overview

**Priority:** P1 · **Status:** pending · **Effort:** ~48h
Build the complete React frontend: app shell with routing/theme, EN+VI i18n (switchable, persisted), the compare wizard, a virtualized side-by-side diff view with inline word/char diff rendering and move indicators, a filterable change list with severity badges, audit-trail detail panels, the LLM explanation panel with streaming display, the download-manager UI, and settings. Acceptance criteria (d) and (e) land here.

## Key Insights

- TanStack Router (code-based routes, SPA mode) + TanStack Query cover navigation and command state; TanStack Virtual keeps 10k-row diffs smooth.
- i18next 27 is a breaking major — greenfield setup with `react-i18next`, JSON resources, EN/VI key parity enforced by a unit test.
- Tauri events (`diff://progress`, `llm://token`, `dl://progress`) need unlisten-on-unmount discipline; typed invoke wrappers in one module keep command names in one place.
- Diff rendering maps Phase 3 `word_segments` (insert/delete/equal spans) onto colored spans; formatting-only and Moved changes get distinct visual treatment.

## Requirements

- [ ] Shell: routed pages (Compare wizard, Diff result, History, Settings), dark/light theme, Tauri `ping`-proven command layer.
- [ ] i18n EN+VI with language switcher persisted in localStorage; EN/VI key parity test green.
- [ ] Compare wizard: file pickers (A/B), validation, scanned-PDF notice, options (tracked-changes mode, embedding model, OCR toggle with experimental label), progress streaming, re-run.
- [ ] Side-by-side virtualized view with inline diff spans, moved arrows, sync scroll, context toggle.
- [ ] Change list: filters (op, severity, entity kind, formatting-only), severity badges, click→scroll sync.
- [ ] Audit trail detail per change; LLM explain panel (install prompts + streaming + cancel).
- [ ] Downloads page + settings (language, model variant, OCR flag, data dir, clear history).
- [ ] `pnpm build` and `pnpm test` green.

## Architecture

`src/routes/` (TanStack Router): `__root.tsx`, `index.tsx` (wizard), `compare.$sessionId.tsx` (result), `history.tsx`, `settings.tsx`. `src/lib/tauri.ts` = typed invoke wrappers + event subscription helpers; TS interfaces mirroring core serde types (camelCase). `src/components/compare/` holds diff view pieces; `src/components/ui/` shadcn primitives; `src/lib/i18n.ts` bootstraps i18next with `src/i18n/en.json` + `src/i18n/vi.json`. State: TanStack Query for commands; Zustand-free — local component state + query cache suffice.

## Related Code Files

- `src/main.tsx`, `src/App.tsx`, `src/styles/globals.css` (modify — shell + theme tokens)
- `src/routes/__root.tsx`, `src/routes/index.tsx`, `src/routes/compare.$sessionId.tsx`, `src/routes/history.tsx`, `src/routes/settings.tsx` (create)
- `src/components/compare/FilePicker.tsx`, `OptionsForm.tsx`, `ProgressPanel.tsx` (create)
- `src/components/compare/SideBySideView.tsx`, `InlineDiffSegment.tsx`, `MoveArrow.tsx` (create)
- `src/components/compare/ChangeList.tsx`, `SeverityBadge.tsx`, `AuditTrailPanel.tsx`, `LlmExplainPanel.tsx` (create)
- `src/components/downloads/DownloadJobs.tsx` (create)
- `src/components/ui/*` (create — shadcn button/dialog/select/badge/switch/tabs)
- `src/lib/tauri.ts`, `src/lib/types.ts`, `src/lib/severity.ts`, `src/lib/i18n.ts` (create)
- `src/i18n/en.json`, `src/i18n/vi.json` (create)
- `src-tauri/src/commands.rs` (modify — align TS-facing return shapes with `src/lib/types.ts`; `run_diff`/`get_diff`/`cancel_diff` were created in Phase 3, `ocr_status` added in Phase 6 — this phase modifies only)
- `src/lib/__tests__/diffMapping.test.ts`, `src/lib/__tests__/i18nParity.test.ts` (create)

## Implementation Steps

1. Typed command layer: `src/lib/types.ts` mirroring `DiffResult`, `Change`, `AuditEntry`, download/LLM status types (camelCase serde already aligned; gains `Format::PdfOcr`/`StyleHints.ocr` mirrors when Phase 6 lands); `src/lib/tauri.ts` with `invoke` wrappers (`runDiff`, `getDiff`, `cancelDiff`, `llmStatus`, `llmInstallServer`, `llmDownloadModel`, `llmExplain`, `llmCancel`, `ocrStatus`) and `listen` helpers returning cleanup functions. The session/export wrappers (`listSessions`, `deleteSession`, `exportReport`) are deliberately NOT added here — their commands land in Phase 8, which extends `tauri.ts`.
2. Shell + routing: TanStack Router code routes with a persistent left nav; Tailwind 4 `@theme` tokens for both themes; shadcn components initialized (Radix flag).
3. i18n: `i18next` + `react-i18next` + `i18next-resources-to-backend`; `en.json`/`vi.json` full key sets (wizard, diff, severity labels, audit, downloads, settings, errors, OCR experimental notice); switcher in settings + titlebar; persisted; `<html lang>` synced; parity test asserts identical key trees.
4. Compare wizard: two `FilePicker`s using dialog plugin with per-format accept filters; client-side extension validation; option controls (TrackedChangesMode select, embedding model select — default e5-small, bge-m3 strictly opt-in download-only and disabled until downloaded, OCR switch rendered with the "Experimental" badge + honest description, page-cap note); Start → `runDiff` with progress bar from `diff://progress` events; error state surfaces extraction warnings incl. `ScannedPdfDetected` notice.
5. Side-by-side view: `TanStack Virtual` over pair rows (only changed + context); columns A|B with `InlineDiffSegment` coloring insert/delete spans, `MoveArrow` linking moved pairs; toolbar: severity/op filters, context toggle, sync-scroll toggle; click row ⇄ highlight.
6. Change list: grouped by op with `SeverityBadge` (Critical/High/Medium/Low colors from `src/lib/severity.ts`); filters for entity kind + formatting-only; row click scrolls the side-by-side to the pair; summary counts header.
7. Audit trail: expandable detail under each change listing `AuditEntry` rows (rule id, localized rationale via key, matched span excerpt, points) — the contract's audit-trail UI.
8. LLM panel: status from `llmStatus`; if server/model missing show install buttons wired to download jobs (progress from `dl://progress`); "Explain changes" → `llmExplain`, tokens streamed from `llm://token` into a scroll-locked panel, cancel button, parsed summary/explanations/risk-note rendered as plain text only — React escaping, no markdown-HTML rendering, explanation length capped — under an explicit "AI-generated, may be wrong" label, visually distinct from deterministic audit entries; LLM output is never persisted or exported into reports without that label; prompt language follows app language.
9. Downloads + settings pages: job list with progress/cancel/verify-state; settings: language, model variant (bge-m3 strictly opt-in download-only, never bundled, with RAM/latency warning), OCR flag, open-data-dir button, clear-history with confirm dialog (wired to Phase 8 commands as they land — stub behind feature flag until then).
10. Tests + polish: vitest for diff-segment→span mapping, severity color mapping, i18n parity; keyboard navigation pass on wizard and lists; empty/error/loading states for every async surface.

## Todo

- [ ] Typed tauri.ts + types.ts over all commands/events
- [ ] Shell, routes, theme, shadcn primitives
- [ ] i18n EN+VI full key sets + parity test
- [ ] Wizard with options + progress + scanned notice
- [ ] Virtualized side-by-side view with inline spans + move arrows
- [ ] Change list with filters + severity badges + audit trail detail
- [ ] LLM panel with install flow + streaming + cancel
- [ ] Downloads + settings pages
- [ ] `pnpm build` green, `pnpm test` green

## Success Criteria

- `pnpm build` and `pnpm test` exit 0.
- Manual E2E on the dev VM (GUI session or `xvfb-run` per the `docs/dev-setup.md` display note from Phase 1): load two DOCX fixtures → run diff → side-by-side view renders, change list shows severity, audit trail expands, toggling EN↔VI switches all strings (acceptance d partial + e).
- Same for two PDF fixtures; with model installed, explanation streams and renders (acceptance d complete).
- `pnpm exec tsc --noEmit` exits 0.

## Risk Assessment

- i18next 27 API drift vs docs → greenfield setup against v27 migration guide; parity test catches missing keys.
- Virtualization edge cases (very long blocks, CJK/VN wrapping) → cap rendered segment length with expand affordance; visual QA on fixtures.
- Event-listener leaks → `tauri.ts` listen helpers return unlisten; routes clean up in effects (code-review checklist item).

## Security Considerations

- No direct fetch anywhere in `src/` (egress audit covers TS too); all privileged operations go through typed Tauri commands.
- LLM output is untrusted content: rendered as plain text via React escaping (never markdown-HTML), capped, and labeled "AI-generated, may be wrong"; never mixed into audit-trail entries or exported into reports without that label.
- File paths displayed but never sent to third parties; capabilities file grants only needed plugins/permissions.

## Next Steps

Phase 8 replaces the stubbed history/clear-history surfaces with real rusqlite persistence and adds report export.
