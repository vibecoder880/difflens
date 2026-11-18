//! Theme store: system/light/dark, persisted, `.dark` class on <html>,
//! live-synced with `prefers-color-scheme` when in System mode.

import { useSyncExternalStore } from "react";

export type ThemeMode = "system" | "light" | "dark";

const STORAGE_KEY = "difflens.theme";
export const THEME_MODES: readonly ThemeMode[] = ["system", "light", "dark"];

let mode: ThemeMode = loadMode();
const listeners = new Set<() => void>();

const media =
  typeof window !== "undefined" && "matchMedia" in window
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

function loadMode(): ThemeMode {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === "system" || stored === "light" || stored === "dark") return stored;
  } catch {
    // Ignore unavailable localStorage.
  }
  return "system";
}

function persist(next: ThemeMode): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Non-fatal.
  }
}

function systemPrefersDark(): boolean {
  return media?.matches ?? false;
}

/** Resolve the mode against the OS preference. */
export function resolvedDark(m: ThemeMode = mode): boolean {
  return m === "dark" || (m === "system" && systemPrefersDark());
}

function apply(): void {
  document.documentElement.classList.toggle("dark", resolvedDark());
  document.documentElement.style.colorScheme = resolvedDark() ? "dark" : "light";
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function setMode(next: ThemeMode): void {
  mode = next;
  persist(next);
  apply();
  for (const fn of listeners) fn();
}

function onSystemChange(): void {
  if (mode === "system") {
    apply();
    for (const fn of listeners) fn();
  }
}

/** Initialize: apply the stored mode and follow OS changes. Idempotent. */
export function initTheme(): void {
  apply();
  media?.addEventListener("change", onSystemChange);
}

export function getThemeMode(): ThemeMode {
  return mode;
}

/** React binding: re-renders on mode or OS-preference changes. */
export function useTheme(): { mode: ThemeMode; setMode: (m: ThemeMode) => void } {
  // Snapshot changes whenever `mode` changes; system flips notify listeners.
  useSyncExternalStore(
    subscribe,
    () => mode,
    () => "system" as ThemeMode,
  );
  return { mode, setMode };
}
