---
title: "Phase 4: Heuristic intelligence (VN/EN entities, severity, audit trail)"
status: todo
priority: P1
effort: 24h
---

# Phase 4: Heuristic Intelligence Layer

## Context Links

- Plan: [plan.md](./plan.md)
- Research: §2.8 (strsim for similarity scores feeding severity); brainstorm contract: deterministic money/date/percent/duration regex + severity rules + audit trail.

## Overview

**Priority:** P1 · **Status:** pending · **Effort:** ~24h
Add the deterministic intelligence layer on top of Phase 3's `DiffResult`: VN+EN entity extraction (money, date, percent, duration), old→new delta computation, a severity lexicon + scoring engine, and a serializable audit trail explaining every score. 100% rule-based and deterministic — same input always yields the same output (golden tests enforce it).

## Key Insights

- Vietnamese and English number formats differ (VN: `1.000.000đ` thousands-dot, `1,5` decimal comma; EN: `1,000,000.00`) — regex set must handle both without misparsing; currency tokens: `₫, VNĐ, VND, đ, đồng, $, USD, €, EUR`.
- Date formats to cover: `dd/mm/yyyy`, `dd-mm-yyyy`, `yyyy-mm-dd`, `ngày dd tháng mm năm yyyy`, `March 3, 2026`, `3 March 2026`.
- Severity scoring: lexical risk terms (payment/liability/termination/warranty/penalty + thanh toán/trách nhiệm/bảo hành/chấm dứt/phạt/hủy/bồi thường/hiệu lực/bảo mật/hạn) in changed regions raise points; entity deltas (money/percent/date) multiply points; banded Low/Medium/High/Critical.
- The audit trail is a first-class product feature (brainstorm acceptance criterion d): every point contribution is an `AuditEntry {rule_id, matched_span, points, rationale}` attached to its Change.

## Requirements

- [ ] Entity extraction for Money/Date/Percent/Duration with VN+EN formats, positions in raw text.
- [ ] Delta computation: numeric absolute/relative, percent-point delta, date difference in days.
- [ ] Severity lexicon + scoring rules producing 0–100 points → Critical/High/Medium/Low bands.
- [ ] `AuditEntry`/`AuditTrail` serde types attached per Change; full JSON serializable for UI + reports.
- [ ] Determinism proven by golden-file tests; `cargo test -p difflens-core heuristic` green.

## Architecture

`core/src/heuristic/entities.rs`: ordered regex set + `Entity {kind, text, byte_span, parsed: ParsedValue}` where `ParsedValue` is Money(f64, currency), Date(NaiveDate), Percent(f64), Duration(f64, unit); parse into canonical values (currency-aware, VN/EN separator-aware). `core/src/heuristic/delta.rs`: `EntityDelta {kind, old, new, abs_delta, rel_delta, days}` for paired entities across a Replace pair. `core/src/heuristic/severity.rs`: static ordered rule table `Rule {id, matcher, points, rationale_key}` — rules match change text + deltas, sum points (clamped 0–100), band via thresholds; i18n-ready rationale keys (Phase 7 maps keys to EN/VI strings). `core/src/heuristic/audit.rs`: `AuditEntry`, `AuditTrail`, attach to `Change.audit` (extending Phase 3 types).

## Related Code Files

- `core/src/heuristic/mod.rs` (create — orchestration: enrich DiffResult)
- `core/src/heuristic/entities.rs` (create — regex set + parsing)
- `core/src/heuristic/delta.rs` (create — entity deltas)
- `core/src/heuristic/severity.rs` (create — lexicon, rules, scoring, bands)
- `core/src/heuristic/audit.rs` (create — AuditEntry/AuditTrail serde types)
- `src-tauri/src/commands.rs` (modify — `run_diff` pipeline from Phase 3 now ends with enrich; `diff://progress` gains the enrich stage)
- `src-tauri/src/state.rs` (modify — job results now carry enriched changes)
- `core/tests/heuristic_tests.rs`, `core/tests/golden/*.json` (create)

## Implementation Steps

1. `entities.rs`: build the ordered regex set (money VN/EN, dates 6 patterns, percent, duration VN/EN units) with `once_cell`-static RegexSet; parse each match into canonical values handling VN thousands-dot vs EN comma ambiguity via currency/locale context from the match itself; unit table tests VN+EN.
2. `delta.rs`: pair entities between old/new texts of a Replace pair (same kind, nearest match); compute abs/relative deltas, percent-point change, day deltas; emit `EntityDelta` list per change.
3. `severity.rs`: encode the rule table — base lexical rules (per lexicon above, matched on changed word segments only), entity-delta multiplier rules (money/date/percent changes scale points by magnitude bands), formatting-only demotion; sum + clamp; band thresholds Critical ≥80, High ≥60, Medium ≥30 else Low; each rule writes an `AuditEntry` with i18n rationale key + matched span.
4. `audit.rs`: serde types (`#[serde(rename_all = "camelCase")]` for TS friendliness); extend Phase 3 `Change` with `severity: Severity`, `score: u8`, `audit: Vec<AuditEntry>`; `AuditTrail` aggregates per session.
5. `mod.rs`: `enrich(diff: &mut DiffResult) -> AuditTrail` wiring extraction→delta→scoring for every non-Equal change; deterministic rule ordering (table order, no HashMap iteration in scoring).
6. Golden tests: run enrich over fixture diff results, compare full serialized JSON against `core/tests/golden/*.json`; table tests for each rule (hit + miss cases); determinism test (two runs, byte-identical JSON); VN-specific tests (đ formatting, `ngày … tháng … năm …` dates, duration units).
7. Wire the whole pipeline smoke: `extract → diff → align → enrich` in a single test asserting severity summary counts on the Phase 2 DOCX fixture pair; modify the Phase 3 `run_diff` command so the app-facing pipeline ends with enrich before returning, and extend its `diff://progress` stream with the enrich stage.

## Todo

- [ ] Entity regexes + canonical parsing (VN+EN) with table tests
- [ ] Delta computation (money/percent/date/duration)
- [ ] Rule table + scoring + bands + demotion rules
- [ ] AuditEntry/AuditTrail serde types attached to Changes
- [ ] Golden tests + determinism test
- [ ] `run_diff` pipeline ends with enrich (commands.rs modified); headless round-trip still green
- [ ] `cargo test -p difflens-core heuristic` green

## Success Criteria

- `cargo test -p difflens-core heuristic` exits 0, including golden-file comparisons.
- Running `enrich` twice over the same fixture diff yields byte-identical JSON (determinism).
- Every non-Equal change in the golden files carries ≥1 AuditEntry with a rationale key and points.
- `run_diff` headless round-trip (Phase 3 test, updated) returns enriched changes — severity bands + ≥1 AuditEntry on the DOCX fixture pair — and `diff://progress` includes the enrich stage.
- `cargo clippy -p difflens-core -- -D warnings` exits 0.

## Risk Assessment

- Regex false positives (e.g., `1.000` as money without currency) → entity kinds require explicit currency/percent/unit tokens; ambiguous numeric-only matches are ignored (documented rule).
- Lexicon over/under-triggering → rule table is data, tunable without touching engine; golden tests lock behavior until intentionally updated.
- Locale-ambiguous decimals (`1,5` vs `1,000`) → context-based disambiguation tests; ambiguous cases parsed conservatively (integer) with audit note.

## Security Considerations

- Pure computation, no I/O beyond the diff result in memory; no network, no filesystem.
- Rule rationale keys only — no user document text embedded in rule definitions.

## Next Steps

Phase 5 adds the LLM explanation feature that consumes the enriched, severity-sorted change list, plus the download manager (sole egress point) it depends on.
