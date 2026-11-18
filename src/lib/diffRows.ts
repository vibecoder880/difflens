//! Pure row-model logic for the virtualized side-by-side view.
//!
//! Turns `DiffResult.changes` into a flat row list: one row per non-equal
//! change, consecutive `equal` changes collapsed into a single expandable
//! context row ("N unchanged blocks"). Collapsing is controlled by an
//! expanded-set so it stays a pure function of (changes, expanded) — easy
//! to unit-test and cheap to recompute.

import type { Change, DiffSpan, SpanOp } from "./types";

export type DiffRow =
  | {
      type: "change";
      /** Stable key for react keys + virtualizer measurements. */
      key: string;
      change: Change;
      /** Index into the original changes array. */
      changeIndex: number;
    }
  | {
      type: "context";
      key: string;
      /** The collapsed equal changes (rendered when expanded). */
      changes: Change[];
      expanded: boolean;
    };

/**
 * Build the virtualizer row model. Equal runs collapse into context rows
 * unless their key is present in `expanded`.
 */
export function buildRows(changes: Change[], expanded: ReadonlySet<string>): DiffRow[] {
  const rows: DiffRow[] = [];
  let equalRun: Change[] = [];

  const flushRun = () => {
    if (equalRun.length === 0) return;
    const key = `ctx:${equalRun[0]?.id ?? "empty"}`;
    rows.push({ type: "context", key, changes: equalRun, expanded: expanded.has(key) });
    equalRun = [];
  };

  for (let i = 0; i < changes.length; i += 1) {
    const change = changes[i];
    if (!change) continue;
    if (change.kind === "equal") {
      equalRun.push(change);
      continue;
    }
    flushRun();
    rows.push({ type: "change", key: `ch:${change.id}`, change, changeIndex: i });
  }
  flushRun();
  return rows;
}

/** Index of the row that contains (or is) the given change — for scroll-to. */
export function rowIndexForChange(rows: DiffRow[], changeId: string): number {
  const direct = rows.findIndex((r) => r.type === "change" && r.change.id === changeId);
  if (direct >= 0) return direct;
  // Inside a collapsed context run: expand nothing, jump to the run start.
  return rows.findIndex(
    (r) => r.type === "context" && r.changes.some((c) => c.id === changeId),
  );
}

/** Every change id referenced by a row (context rows expose all members). */
export function rowChangeIds(row: DiffRow): string[] {
  return row.type === "change" ? [row.change.id] : row.changes.map((c) => c.id);
}

/**
 * Split inline word spans into the two column views:
 * old column keeps equal+delete, new column keeps equal+insert.
 */
export function sidesFromSpans(spans: DiffSpan[]): {
  old: DiffSpan[];
  new: DiffSpan[];
} {
  const old: DiffSpan[] = [];
  const newSide: DiffSpan[] = [];
  for (const span of spans) {
    if (span.op === "equal" || span.op === "delete") old.push(span);
    if (span.op === "equal" || span.op === "insert") newSide.push(span);
  }
  return { old, new: newSide };
}

/** Merge adjacent spans sharing the same op (word diffs fragment heavily). */
export function mergeAdjacentSpans(spans: DiffSpan[]): DiffSpan[] {
  const merged: DiffSpan[] = [];
  for (const span of spans) {
    const last = merged[merged.length - 1];
    if (last && last.op === span.op) {
      merged[merged.length - 1] = { op: last.op as SpanOp, text: last.text + span.text };
    } else {
      merged.push({ ...span });
    }
  }
  return merged;
}

/** Full text of one side (for the detail drawer). */
export function spansText(spans: DiffSpan[]): string {
  return spans.map((s) => s.text).join("");
}

/**
 * Full text carried for side-only changes: Added exposes `new_text`,
 * Removed exposes `old_text`. Paired changes (Modified/Moved/Equal) render
 * spans instead and return null here.
 */
export function sideOnlyText(change: Change, side: "old" | "new"): string | null {
  const text = side === "old" ? change.old_text : change.new_text;
  return typeof text === "string" && text.length > 0 ? text : null;
}

/**
 * Single-line excerpt for the change list: inline span text when present,
 * else the carried side-only text (Added/Removed), else the block ids.
 */
export function changeExcerpt(change: Change, maxLen = 90): string {
  const text = change.spans
    .map((s) => s.text)
    .join("")
    .trim()
    .replaceAll(/\s+/g, " ");
  const flat =
    text.length > 0
      ? text
      : (sideOnlyText(change, "old") ?? sideOnlyText(change, "new") ?? "")
          .trim()
          .replaceAll(/\s+/g, " ");
  if (flat.length > 0) return flat.length > maxLen ? `${flat.slice(0, maxLen)}…` : flat;
  const ids = [...change.old_block_ids, ...change.new_block_ids];
  return ids.length > 0 ? ids.join(" ") : change.id;
}
