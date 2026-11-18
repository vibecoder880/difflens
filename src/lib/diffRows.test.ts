//! Pure-logic tests for the virtualizer row model (src/lib/diffRows.ts).

import { describe, expect, it } from "vitest";
import {
  buildRows,
  changeExcerpt,
  mergeAdjacentSpans,
  rowIndexForChange,
  rowChangeIds,
  sideOnlyText,
  sidesFromSpans,
  spansText,
} from "./diffRows";
import type { Change, ChangeKind, DiffSpan } from "./types";

function change(id: string, kind: ChangeKind, overrides: Partial<Change> = {}): Change {
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
    severity: null,
    flags: [],
    ...overrides,
  };
}

describe("buildRows", () => {
  const changes = [
    change("c0", "equal"),
    change("c1", "equal"),
    change("c2", "modified"),
    change("c3", "equal"),
    change("c4", "added"),
    change("c5", "equal"),
    change("c6", "equal"),
    change("c7", "equal"),
  ];

  it("collapses every equal run into one context row", () => {
    const rows = buildRows(changes, new Set());
    expect(rows.map((r) => r.type)).toEqual([
      "context",
      "change",
      "context",
      "change",
      "context",
    ]);
    const [firstCtx] = rows.filter((r) => r.type === "context");
    if (firstCtx?.type !== "context") throw new Error("expected context row");
    expect(firstCtx.changes.map((c) => c.id)).toEqual(["c0", "c1"]);
    expect(firstCtx.expanded).toBe(false);
  });

  it("expands a run when its key is in the expanded set", () => {
    const expanded = new Set(["ctx:c5"]);
    const rows = buildRows(changes, expanded);
    const expandedRun = rows.find((r) => r.type === "context" && r.key === "ctx:c5");
    if (expandedRun?.type !== "context") throw new Error("expected ctx:c5");
    expect(expandedRun.expanded).toBe(true);
    expect(expandedRun.changes.map((c) => c.id)).toEqual(["c5", "c6", "c7"]);
  });

  it("returns an empty list for an all-equal result only when collapsed counts as one row", () => {
    const allEqual = [change("c0", "equal"), change("c1", "equal")];
    expect(buildRows(allEqual, new Set())).toHaveLength(1);
  });

  it("produces one row per change when nothing is equal", () => {
    const rows = buildRows([change("a", "added"), change("b", "removed")], new Set());
    expect(rows).toHaveLength(2);
    expect(rows.every((r) => r.type === "change")).toBe(true);
  });

  it("keeps keys stable and unique across rebuilds", () => {
    const a = buildRows(changes, new Set());
    const b = buildRows(changes, new Set(["ctx:c5"]));
    const keysA = a.map((r) => r.key);
    const keysB = b.map((r) => r.key);
    expect(new Set(keysA).size).toBe(keysA.length);
    // The expanded run reuses the same key so virtualizer measurements survive.
    for (const key of keysA) {
      if (b.some((r) => r.type === "context" && r.key === key)) {
        expect(keysB).toContain(key);
      }
    }
  });
});

describe("rowIndexForChange", () => {
  const changes = [
    change("c0", "equal"),
    change("c1", "equal"),
    change("c2", "modified"),
    change("c3", "equal"),
  ];
  const collapsed = buildRows(changes, new Set());
  const expanded = buildRows(changes, new Set(["ctx:c0"]));

  it("finds direct change rows", () => {
    expect(rowIndexForChange(collapsed, "c2")).toBe(1);
  });

  it("falls back to the containing context run", () => {
    expect(rowIndexForChange(collapsed, "c3")).toBe(2);
  });

  it("maps a member of an expanded run to that run's row", () => {
    // Both c0 and c1 live in the same expanded context row (index 0).
    expect(rowIndexForChange(expanded, "c1")).toBe(0);
  });

  it("returns -1 for unknown ids", () => {
    expect(rowIndexForChange(collapsed, "nope")).toBe(-1);
  });
});

describe("rowChangeIds", () => {
  it("exposes member ids of context runs for selection", () => {
    const rows = buildRows(
      [change("c0", "equal"), change("c1", "equal"), change("c2", "moved")],
      new Set(),
    );
    const ctx = rows[0];
    if (ctx?.type !== "context") throw new Error("expected context row");
    expect(rowChangeIds(ctx)).toEqual(["c0", "c1"]);
    const ch = rows[1];
    if (ch?.type !== "change") throw new Error("expected change row");
    expect(rowChangeIds(ch)).toEqual(["c2"]);
  });
});

describe("sidesFromSpans", () => {
  const spans: DiffSpan[] = [
    { op: "equal", text: "Pay within " },
    { op: "delete", text: "30" },
    { op: "insert", text: "45" },
    { op: "equal", text: " days." },
  ];

  it("old side keeps equal + delete", () => {
    const { old: oldSide } = sidesFromSpans(spans);
    expect(oldSide.map((s) => s.op)).toEqual(["equal", "delete", "equal"]);
    expect(spansText(oldSide)).toBe("Pay within 30 days.");
  });

  it("new side keeps equal + insert", () => {
    const { new: newSide } = sidesFromSpans(spans);
    expect(newSide.map((s) => s.op)).toEqual(["equal", "insert", "equal"]);
    expect(spansText(newSide)).toBe("Pay within 45 days.");
  });
});

describe("mergeAdjacentSpans", () => {
  it("fuses neighbouring spans with the same op", () => {
    const spans: DiffSpan[] = [
      { op: "equal", text: "a " },
      { op: "equal", text: "b " },
      { op: "delete", text: "x" },
      { op: "insert", text: "y" },
      { op: "insert", text: "z" },
    ];
    expect(mergeAdjacentSpans(spans)).toEqual([
      { op: "equal", text: "a b " },
      { op: "delete", text: "x" },
      { op: "insert", text: "yz" },
    ]);
  });

  it("keeps single spans and empty input intact", () => {
    expect(mergeAdjacentSpans([{ op: "equal", text: "hi" }])).toEqual([
      { op: "equal", text: "hi" },
    ]);
    expect(mergeAdjacentSpans([])).toEqual([]);
  });
});

describe("sideOnlyText", () => {
  it("exposes new_text for added changes", () => {
    const c = change("a", "added", { new_text: "Extra clause." });
    expect(sideOnlyText(c, "new")).toBe("Extra clause.");
    expect(sideOnlyText(c, "old")).toBeNull();
  });

  it("exposes old_text for removed changes", () => {
    const c = change("r", "removed", { old_text: "Liability cap of 1.000 USD." });
    expect(sideOnlyText(c, "old")).toBe("Liability cap of 1.000 USD.");
    expect(sideOnlyText(c, "new")).toBeNull();
  });

  it("returns null for paired kinds and missing text", () => {
    expect(sideOnlyText(change("m", "modified"), "old")).toBeNull();
    expect(sideOnlyText(change("e", "equal"), "new")).toBeNull();
    expect(sideOnlyText(change("x", "added"), "new")).toBeNull();
  });
});

describe("changeExcerpt", () => {
  it("uses inline span text for modified changes", () => {
    const c = change("m", "modified", {
      spans: [
        { op: "equal", text: "Pay within " },
        { op: "insert", text: "45" },
        { op: "delete", text: "30" },
        { op: "equal", text: " days." },
      ],
    });
    expect(changeExcerpt(c)).toBe("Pay within 4530 days.");
  });

  it("uses the carried block text for added/removed changes", () => {
    expect(changeExcerpt(change("a", "added", { new_text: "  Extra   clause. " }))).toBe(
      "Extra clause.",
    );
    expect(changeExcerpt(change("r", "removed", { old_text: "Governing law of Vietnam." }))).toBe(
      "Governing law of Vietnam.",
    );
  });

  it("falls back to block ids and the change id", () => {
    expect(changeExcerpt(change("a", "added", { new_block_ids: ["b2", "b3"] }))).toBe("b2 b3");
    expect(changeExcerpt(change("solo", "modified"))).toBe("solo");
  });

  it("truncates long text with an ellipsis at 90 chars", () => {
    const long = "x".repeat(120);
    const excerpt = changeExcerpt(change("a", "added", { new_text: long }));
    expect(excerpt).toHaveLength(91);
    expect(excerpt.endsWith("…")).toBe(true);
  });
});
