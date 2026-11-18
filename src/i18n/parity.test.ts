//! EN/VI i18n key parity, enforced against the backend label-key lists.
//!
//! The authoritative lists live in Rust (`core/src/heuristic/labels.rs`
//! ALL_LABEL_KEYS, `core/src/ocr/labels.rs` OCR_LABEL_KEYS) and are
//! exported to `src/i18n/label-keys.json` by `scripts/export-labels.mjs`
//! (runs on predev/prebuild and via `npm test`). Both translation files
//! must have identical key trees, and both must cover every backend label
//! key — adding a rule in Rust without UI strings now breaks the build.

import { describe, expect, it } from "vitest";
import en from "./en.json";
import vi from "./vi.json";
import labelKeys from "./label-keys.json";

type JsonTree = Record<string, unknown>;

/** Flatten a nested JSON object into a sorted "a.b.c" key list. */
function flattenKeys(tree: JsonTree, prefix = ""): string[] {
  const keys: string[] = [];
  for (const [key, value] of Object.entries(tree)) {
    const path = prefix.length > 0 ? `${prefix}.${key}` : key;
    if (value !== null && typeof value === "object" && !Array.isArray(value)) {
      keys.push(...flattenKeys(value as JsonTree, path));
    } else {
      keys.push(path);
    }
  }
  return keys.sort();
}

const enKeys = flattenKeys(en as JsonTree);
const viKeys = flattenKeys(vi as JsonTree);

describe("i18n parity", () => {
  it("en and vi expose identical key sets", () => {
    const enOnly = enKeys.filter((k) => !viKeys.includes(k));
    const viOnly = viKeys.filter((k) => !enKeys.includes(k));
    expect(enOnly, `keys missing in vi.json: ${enOnly.join(", ")}`).toEqual([]);
    expect(viOnly, `keys missing in en.json: ${viOnly.join(", ")}`).toEqual([]);
  });

  it("en keys are unique (no flattening collisions)", () => {
    expect(new Set(enKeys).size).toBe(enKeys.length);
  });

  it("covers every heuristic label key (ALL_LABEL_KEYS)", () => {
    const missingEn = labelKeys.heuristic.filter((k) => !enKeys.includes(k));
    const missingVi = labelKeys.heuristic.filter((k) => !viKeys.includes(k));
    expect(missingEn, "missing in en.json").toEqual([]);
    expect(missingVi, "missing in vi.json").toEqual([]);
  });

  it("covers every OCR label key (OCR_LABEL_KEYS)", () => {
    const missingEn = labelKeys.ocr.filter((k) => !enKeys.includes(k));
    const missingVi = labelKeys.ocr.filter((k) => !viKeys.includes(k));
    expect(missingEn, "missing in en.json").toEqual([]);
    expect(missingVi, "missing in vi.json").toEqual([]);
  });

  it("label-key export is non-empty (guards a broken grep)", () => {
    expect(labelKeys.heuristic.length).toBeGreaterThan(0);
    expect(labelKeys.ocr.length).toBeGreaterThan(0);
  });
});
