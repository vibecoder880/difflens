//! i18next bootstrap: EN + VI resources, OS-locale detection, localStorage
//! persistence, `<html lang>` sync. Import `initI18n()` before rendering.

import i18next, { type i18n as I18n } from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./en.json";
import vi from "./vi.json";

export const LOCALES = ["en", "vi"] as const;
export type Locale = (typeof LOCALES)[number];

export const DEFAULT_LOCALE: Locale = "en";

const STORAGE_KEY = "difflens.locale";

function isLocale(value: string): value is Locale {
  return (LOCALES as readonly string[]).includes(value);
}

/** Stored preference wins; otherwise follow the OS locale (vi → vi, else en). */
export function detectLocale(): Locale {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored && isLocale(stored)) return stored;
  } catch {
    // localStorage unavailable (rare embedded webviews) — fall through.
  }
  const os = typeof navigator !== "undefined" ? navigator.language : "";
  return os.toLowerCase().startsWith("vi") ? "vi" : DEFAULT_LOCALE;
}

export function persistLocale(locale: Locale): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // Non-fatal: preference just won't survive restarts.
  }
  document.documentElement.lang = locale;
}

/** Idempotent init — safe to call from multiple entry points. */
export function initI18n(): Promise<I18n> {
  const lng = detectLocale();
  if (i18next.isInitialized) {
    return Promise.resolve(i18next);
  }
  void persistLocale(lng);
  return i18next.use(initReactI18next).init({
    lng,
    fallbackLng: DEFAULT_LOCALE,
    resources: {
      en: { translation: en },
      vi: { translation: vi },
    },
    interpolation: { escapeValue: false },
    returnNull: false,
  });
}

export function isLocaleValue(value: string): value is Locale {
  return isLocale(value);
}
