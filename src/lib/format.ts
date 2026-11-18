//! Pure formatting helpers: severity styling, deltas, numbers, dates.
//! No React, no Tauri — fully unit-testable (see format.test.ts).

import type { ChangeKind, DocFormat, Severity } from "./types";

// ---------------------------------------------------------------------------
// Severity — one palette entry per band (tokens defined in index.css @theme).
// ---------------------------------------------------------------------------

export interface SeverityStyle {
  /** Text color class (AA contrast on the badge surface). */
  text: string;
  /** Badge background class. */
  bg: string;
  /** Badge border class. */
  border: string;
  /** Solid dot color class for indicators. */
  dot: string;
}

export const SEVERITY_STYLES: Record<Severity, SeverityStyle> = {
  info: {
    text: "text-severity-info",
    bg: "bg-severity-info-soft",
    border: "border-severity-info-edge",
    dot: "bg-severity-info",
  },
  low: {
    text: "text-severity-low",
    bg: "bg-severity-low-soft",
    border: "border-severity-low-edge",
    dot: "bg-severity-low",
  },
  medium: {
    text: "text-severity-medium",
    bg: "bg-severity-medium-soft",
    border: "border-severity-medium-edge",
    dot: "bg-severity-medium",
  },
  high: {
    text: "text-severity-high",
    bg: "bg-severity-high-soft",
    border: "border-severity-high-edge",
    dot: "bg-severity-high",
  },
  critical: {
    text: "text-severity-critical",
    bg: "bg-severity-critical-soft",
    border: "border-severity-critical-edge",
    dot: "bg-severity-critical",
  },
};

export function severityStyle(severity: Severity): SeverityStyle {
  return SEVERITY_STYLES[severity];
}

/** Severity display order (critical first). */
export const SEVERITY_ORDER: readonly Severity[] = [
  "critical",
  "high",
  "medium",
  "low",
  "info",
];

// ---------------------------------------------------------------------------
// Delta formatting (HeuristicFlag.delta: -0.5 ⇒ −50 %)
// ---------------------------------------------------------------------------

/** Signed relative delta as a percentage string, e.g. "-50%". */
export function formatDelta(delta: number | null | undefined): string {
  if (delta === null || delta === undefined || !Number.isFinite(delta)) {
    return "—";
  }
  const percent = Math.round(delta * 1000) / 10;
  const sign = percent > 0 ? "+" : percent < 0 ? "−" : "±";
  const abs = Math.abs(percent);
  const body = Number.isInteger(abs) ? String(abs) : abs.toFixed(1);
  return `${sign}${body}%`;
}

/** Formatted byte size, e.g. "2.3 GB" (locale-independent, fixed decimals). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 100 || unit === 0 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

// ---------------------------------------------------------------------------
// Numbers / dates (locale aware)
// ---------------------------------------------------------------------------

export function formatNumber(value: number, locale: string): string {
  return new Intl.NumberFormat(locale).format(value);
}

/** Date-time in the given locale; returns the raw string when unparseable. */
export function formatDateTime(input: string | number | Date, locale: string): string {
  const date = input instanceof Date ? input : new Date(input);
  if (Number.isNaN(date.getTime())) return String(input);
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

/**
 * Relative time ("3 minutes ago", "2 days ago") via Intl.RelativeTimeFormat.
 * Future timestamps clamp to "just now". Falls back to the absolute date
 * when the runtime lacks Intl.RelativeTimeFormat.
 */
export function formatRelativeTime(epochMs: number, locale: string, now: number = Date.now()): string {
  const date = new Date(epochMs);
  if (Number.isNaN(date.getTime())) return String(epochMs);
  if (typeof Intl === "undefined" || typeof Intl.RelativeTimeFormat !== "function") {
    return formatDateTime(date, locale);
  }
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const diffMs = epochMs - now;
  const absMs = Math.abs(diffMs);
  // Future timestamps (clock skew) or anything under a minute → "now".
  if (diffMs > 0 || absMs < 60_000) {
    return rtf.format(0, "second");
  }
  const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 365 * 24 * 60 * 60_000],
    ["month", 30 * 24 * 60 * 60_000],
    ["week", 7 * 24 * 60 * 60_000],
    ["day", 24 * 60 * 60_000],
    ["hour", 60 * 60_000],
    ["minute", 60_000],
  ];
  for (const [unit, ms] of UNITS) {
    if (absMs >= ms) {
      return rtf.format(-Math.floor(absMs / ms), unit);
    }
  }
  return rtf.format(0, "second");
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

export function fileBaseName(path: string): string {
  const normalized = path.replaceAll("\\", "/");
  const base = normalized.slice(normalized.lastIndexOf("/") + 1);
  return base.length > 0 ? base : path;
}

/** Mirror of `DocFormat::from_extension` (model.rs) for the wizard badges. */
export function formatFromExtension(path: string): DocFormat | null {
  const base = fileBaseName(path);
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return null;
  const ext = base.slice(dot + 1).toLowerCase();
  switch (ext) {
    case "pdf":
      return "pdf";
    case "docx":
      return "docx";
    case "txt":
    case "md":
      return "txt";
    case "xlsx":
    case "xlsm":
      return "xlsx";
    case "pptx":
      return "pptx";
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Change kinds
// ---------------------------------------------------------------------------

/** Chip glyph per change kind (+added −removed ~modified ⇄moved =equal). */
export const KIND_GLYPHS: Record<ChangeKind, string> = {
  added: "+",
  removed: "−",
  modified: "~",
  moved: "⇄",
  equal: "=",
};

export const KIND_ORDER: readonly ChangeKind[] = [
  "modified",
  "added",
  "removed",
  "moved",
  "equal",
];

/** Signed integer with unicode minus, e.g. "-3" → "−3" (tabular-nums). */
export function formatSignedCount(value: number): string {
  const sign = value > 0 ? "+" : value < 0 ? "−" : "";
  return `${sign}${Math.abs(value)}`;
}
