//! Filter state + reducers for the change list (pure, unit-tested).

import { KIND_ORDER, SEVERITY_ORDER } from "./format";
import type { Change, ChangeKind, Severity } from "./types";

export interface DiffFilters {
  kinds: ChangeKind[];
  severities: Severity[];
}

export const EMPTY_FILTERS: DiffFilters = { kinds: [], severities: [] };

/** Immutable toggle: add to the list when absent, remove when present. */
export function toggleValue<T>(list: readonly T[], value: T): T[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}

/** Empty filter lists mean "show everything". */
export function applyFilters(changes: Change[], filters: DiffFilters): Change[] {
  return changes.filter(
    (c) =>
      (filters.kinds.length === 0 || filters.kinds.includes(c.kind)) &&
      (filters.severities.length === 0 ||
        (c.severity !== null && filters.severities.includes(c.severity))),
  );
}

export function countByKind(changes: Change[]): Record<ChangeKind, number> {
  const counts = { added: 0, removed: 0, modified: 0, moved: 0, equal: 0 };
  for (const c of changes) counts[c.kind] += 1;
  return counts;
}

export function countBySeverity(changes: Change[]): Record<Severity, number> {
  const counts: Record<Severity, number> = { info: 0, low: 0, medium: 0, high: 0, critical: 0 };
  for (const c of changes) {
    if (c.severity !== null) counts[c.severity] += 1;
  }
  return counts;
}

/** Deterministic chip order for the filter UI. */
export const FILTER_KINDS = KIND_ORDER;
export const FILTER_SEVERITIES = SEVERITY_ORDER;
