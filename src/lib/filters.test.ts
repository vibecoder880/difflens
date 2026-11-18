//! Pure-logic tests for the change-list filter reducers (src/lib/filters.ts).

import { describe, expect, it } from "vitest";
import {
  applyFilters,
  countByKind,
  countBySeverity,
  toggleValue,
} from "./filters";
import type { Change, ChangeKind, Severity } from "./types";

function change(id: string, kind: ChangeKind, severity: Severity | null): Change {
  return {
    id,
    kind,
    old_block_ids: [],
    new_block_ids: [],
    old_page: null,
    new_page: null,
    spans: [],
    similarity: null,
    move_delta: null,
    severity,
    flags: [],
  };
}

const sample: Change[] = [
  change("c0", "modified", "high"),
  change("c1", "modified", "medium"),
  change("c2", "added", null),
  change("c3", "removed", "low"),
  change("c4", "moved", "info"),
  change("c5", "equal", null),
  change("c6", "modified", "critical"),
];

describe("toggleValue", () => {
  it("adds absent values and removes present ones", () => {
    expect(toggleValue(["a", "b"] as string[], "c")).toEqual(["a", "b", "c"]);
    expect(toggleValue(["a", "b"] as string[], "b")).toEqual(["a"]);
  });

  it("never mutates the input list", () => {
    const original = ["x"] as string[];
    toggleValue(original, "y");
    expect(original).toEqual(["x"]);
  });
});

describe("applyFilters", () => {
  it("empty filters keep everything", () => {
    expect(applyFilters(sample, { kinds: [], severities: [] })).toHaveLength(
      sample.length,
    );
  });

  it("kind filter intersects with severity filter", () => {
    const out = applyFilters(sample, { kinds: ["modified"], severities: ["high"] });
    expect(out.map((c) => c.id)).toEqual(["c0"]);
  });

  it("severity filter drops unclassified changes", () => {
    const out = applyFilters(sample, { kinds: [], severities: ["low"] });
    expect(out.map((c) => c.id)).toEqual(["c3"]);
  });

  it("severity filter alone with no match returns empty", () => {
    const out = applyFilters(sample, { kinds: ["added"], severities: ["critical"] });
    expect(out).toEqual([]);
  });
});

describe("countByKind", () => {
  it("counts every kind including zero entries", () => {
    expect(countByKind(sample)).toEqual({
      added: 1,
      removed: 1,
      modified: 3,
      moved: 1,
      equal: 1,
    });
  });
});

describe("countBySeverity", () => {
  it("counts only classified changes", () => {
    expect(countBySeverity(sample)).toEqual({
      info: 1,
      low: 1,
      medium: 1,
      high: 1,
      critical: 1,
    });
  });
});
