//! Local preferences persisted in localStorage (no backend storage).
//!
//! Shared by the Settings page (writes the preference) and the compare
//! wizard (reads it to pass `ocr_enabled` to `run_diff`).

/** localStorage key for the experimental-OCR preference. */
export const OCR_KEY = "difflens.ocrPrefer";

/** Read the stored OCR preference; default enabled (backend auto mode). */
export function loadOcrPref(): boolean {
  try {
    const stored = window.localStorage.getItem(OCR_KEY);
    return stored === null ? true : stored === "true";
  } catch {
    return true;
  }
}

/** Persist the OCR preference (best-effort; failures are non-fatal). */
export function persistOcrPref(value: boolean): void {
  try {
    window.localStorage.setItem(OCR_KEY, value ? "true" : "false");
  } catch {
    // Non-fatal.
  }
}
