//! Pure-logic tests for src/lib/format.ts.

import { describe, expect, it } from "vitest";
import {
  KIND_GLYPHS,
  SEVERITY_ORDER,
  SEVERITY_STYLES,
  formatBytes,
  formatDateTime,
  formatDelta,
  formatFromExtension,
  formatRelativeTime,
  severityStyle,
} from "./format";
import type { Severity } from "./types";

describe("severity styling", () => {
  it("has a distinct style entry for every severity band", () => {
    const bands: Severity[] = ["info", "low", "medium", "high", "critical"];
    for (const band of bands) {
      const style = severityStyle(band);
      expect(style.text).toBeTruthy();
      expect(style.bg).toBeTruthy();
      expect(style.dot).toBeTruthy();
    }
  });

  it("keeps all five palettes visually distinct", () => {
    const dots = SEVERITY_ORDER.map((s) => SEVERITY_STYLES[s].dot);
    expect(new Set(dots).size).toBe(5);
  });

  it("orders severities critical-first for filter chips", () => {
    expect(SEVERITY_ORDER[0]).toBe("critical");
    expect(SEVERITY_ORDER).toEqual(["critical", "high", "medium", "low", "info"]);
  });
});

describe("formatDelta", () => {
  it("renders normalized fractions as signed percentages", () => {
    expect(formatDelta(-0.5)).toBe("−50%");
    expect(formatDelta(0.25)).toBe("+25%");
    expect(formatDelta(1)).toBe("+100%");
    expect(formatDelta(0)).toBe("±0%");
  });

  it("rounds to one decimal place", () => {
    expect(formatDelta(0.3333)).toBe("+33.3%");
    expect(formatDelta(-0.1)).toBe("−10%");
  });

  it("shows an em-dash placeholder for absent/invalid deltas", () => {
    expect(formatDelta(null)).toBe("—");
    expect(formatDelta(undefined)).toBe("—");
    expect(formatDelta(Number.NaN)).toBe("—");
    expect(formatDelta(Number.POSITIVE_INFINITY)).toBe("—");
  });
});

describe("formatBytes", () => {
  it("formats common model sizes", () => {
    expect(formatBytes(2497281120)).toBe("2.33 GB");
    expect(formatBytes(2104932768)).toBe("1.96 GB");
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.00 KB");
  });

  it("degrades gracefully on invalid input", () => {
    expect(formatBytes(-5)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
  });
});

describe("formatFromExtension", () => {
  it("mirrors DocFormat::from_extension", () => {
    expect(formatFromExtension("/a/contract.pdf")).toBe("pdf");
    expect(formatFromExtension("C:\\docs\\v2.DOCX")).toBe("docx");
    expect(formatFromExtension("notes.md")).toBe("txt");
    expect(formatFromExtension("sheet.XLSM")).toBe("xlsx");
    expect(formatFromExtension("deck.pptx")).toBe("pptx");
  });

  it("rejects unknown or extension-less names", () => {
    expect(formatFromExtension("image.png")).toBeNull();
    expect(formatFromExtension("README")).toBeNull();
    expect(formatFromExtension("")).toBeNull();
  });
});

describe("formatDateTime", () => {
  it("formats an ISO date per locale", () => {
    const out = formatDateTime("2026-02-01T10:30:00Z", "en");
    expect(out).toMatch(/2026/);
    expect(out).toMatch(/Feb/);
  });

  it("falls back to the raw string when unparseable", () => {
    expect(formatDateTime("not a date", "en")).toBe("not a date");
  });
});

describe("formatRelativeTime", () => {
  const NOW = Date.parse("2026-11-18T12:00:00Z");
  const MIN = 60_000;
  const HOUR = 60 * MIN;
  const DAY = 24 * HOUR;

  it("renders past times with the right unit", () => {
    expect(formatRelativeTime(NOW - 30 * MIN, "en", NOW)).toMatch(/30 minutes ago/);
    expect(formatRelativeTime(NOW - 3 * HOUR, "en", NOW)).toMatch(/3 hours ago/);
    expect(formatRelativeTime(NOW - 2 * DAY, "en", NOW)).toMatch(/2 days ago/);
  });

  it("uses larger units as time grows (numeric:auto idiom)", () => {
    expect(formatRelativeTime(NOW - 40 * DAY, "en", NOW)).toMatch(/month/);
    expect(formatRelativeTime(NOW - 400 * DAY, "en", NOW)).toMatch(/year/);
  });

  it("clamps near-now and future timestamps to 'now'", () => {
    expect(formatRelativeTime(NOW - 10_000, "en", NOW)).toMatch(/now/);
    expect(formatRelativeTime(NOW + 5 * MIN, "en", NOW)).toMatch(/now/); // clock skew
  });

  it("localizes to Vietnamese", () => {
    expect(formatRelativeTime(NOW - 2 * HOUR, "vi", NOW)).toMatch(/2 giờ trước/);
  });

  it("falls back to the raw value for invalid input", () => {
    expect(formatRelativeTime(Number.NaN, "en")).toBe(String(Number.NaN));
  });
});

describe("kind glyphs", () => {
  it("covers every change kind with the contract glyphs", () => {
    expect(KIND_GLYPHS).toEqual({
      added: "+",
      removed: "−",
      modified: "~",
      moved: "⇄",
      equal: "=",
    });
  });
});
