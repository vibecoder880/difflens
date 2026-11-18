// Build-time bridge between the Rust engine and the frontend i18n tests.
//
// The backend owns the authoritative lists of user-facing label keys:
//   - core/src/heuristic/labels.rs  -> ALL_LABEL_KEYS ("flag.*")
//   - core/src/ocr/labels.rs        -> OCR_LABEL_KEYS  ("ocr.*")
// The frontend i18n files (src/i18n/en.json, vi.json) must cover both.
// This script greps those Rust consts into src/i18n/label-keys.json so the
// vitest parity test (src/i18n/parity.test.ts) can assert coverage without
// parsing Rust at test time. It runs automatically via the `predev` /
// `prebuild` npm scripts and is re-run by `npm test`. The generated JSON is
// committed so `npx vitest run` works standalone; it changes only when the
// backend label lists change.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function extractKeys(rsPath, constName) {
  const src = readFileSync(resolve(root, rsPath), "utf8");
  const re = new RegExp(
    `pub const ${constName}:\\s*&\\[&str\\]\\s*=\\s*&\\[([\\s\\S]*?)\\];`,
  );
  const block = re.exec(src)?.[1];
  if (!block) {
    throw new Error(`export-labels: could not find ${constName} in ${rsPath}`);
  }
  const keys = [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (keys.length === 0) {
    throw new Error(`export-labels: ${constName} parsed empty in ${rsPath}`);
  }
  return keys;
}

const out = {
  generatedBy: "scripts/export-labels.mjs — do not edit by hand",
  heuristic: extractKeys("core/src/heuristic/labels.rs", "ALL_LABEL_KEYS"),
  ocr: extractKeys("core/src/ocr/labels.rs", "OCR_LABEL_KEYS"),
};

const dest = resolve(root, "src/i18n/label-keys.json");
mkdirSync(dirname(dest), { recursive: true });
writeFileSync(dest, JSON.stringify(out, null, 2) + "\n");
console.log(
  `export-labels: ${out.heuristic.length} heuristic + ${out.ocr.length} ocr keys -> src/i18n/label-keys.json`,
);
