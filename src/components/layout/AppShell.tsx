//! App shell: slim icon sidebar + topbar (LLM status, locale, theme) and a
//! graceful "requires Tauri runtime" banner when previewing in a browser.

import { useEffect, useState } from "react";
import { Link, Outlet, useLocation } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  Download,
  GitCompare,
  Moon,
  Monitor,
  Settings as SettingsIcon,
  Sun,
  X,
} from "lucide-react";

import { Tooltip } from "@/components/ui/tooltip";
import { cn } from "@/lib/cn";
import { isTauri, llmStatus } from "@/lib/tauri";
import { useTheme, type ThemeMode } from "@/lib/theme";

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

const NAV_ITEMS = [
  { to: "/", icon: GitCompare, labelKey: "nav.compare" },
  { to: "/downloads", icon: Download, labelKey: "nav.downloads" },
  { to: "/settings", icon: SettingsIcon, labelKey: "nav.settings" },
] as const;

function Sidebar() {
  const { t } = useTranslation();
  const location = useLocation();
  return (
    <nav
      aria-label={t("nav.label")}
      className="flex h-full w-12 shrink-0 flex-col items-center gap-1 border-r border-line bg-surface px-1.5 py-3"
    >
      <Link
        to="/"
        className="mb-3 flex h-8 w-8 items-center justify-center rounded-lg bg-accent text-accent-fg"
        aria-label={t("app.name")}
      >
        <GitCompare aria-hidden className="h-4.5 w-4.5" />
      </Link>
      {NAV_ITEMS.map(({ to, icon: Icon, labelKey }) => {
        const active =
          to === "/" ? location.pathname === "/" : location.pathname.startsWith(to);
        return (
          <Tooltip key={to} text={t(labelKey)} className="w-full" as="div">
            <Link
              to={to}
              aria-label={t(labelKey)}
              aria-current={active ? "page" : undefined}
              className={cn(
                "flex h-9 w-9 items-center justify-center rounded-lg transition-colors",
                active
                  ? "bg-accent-soft text-accent"
                  : "text-muted hover:bg-surface-2 hover:text-fg",
              )}
            >
              <Icon aria-hidden className="h-4.5 w-4.5" />
            </Link>
          </Tooltip>
        );
      })}
    </nav>
  );
}

// ---------------------------------------------------------------------------
// Topbar pieces
// ---------------------------------------------------------------------------

/** LLM readiness dot: green when server+model ready, amber partial, grey none. */
function LlmIndicator() {
  const { t } = useTranslation();
  const { data, isPending } = useQuery({
    queryKey: ["llm", "status"],
    queryFn: llmStatus,
    refetchInterval: 15_000,
    staleTime: 10_000,
    // In a plain browser this always fails; keep the UI quiet.
    retry: 0,
  });

  const anyModel = data?.models.some((m) => m.present) ?? false;
  const serverOk = (data?.server_installed || data?.server_on_path) ?? false;
  const state: "pending" | "ready" | "partial" | "missing" = isPending
    ? "pending"
    : serverOk && anyModel
      ? "ready"
      : serverOk || anyModel
        ? "partial"
        : "missing";

  const tip =
    state === "pending"
      ? t("common.loading")
      : state === "ready"
        ? data?.server_running
          ? t("llmDot.running")
          : t("llmDot.ready")
        : state === "partial"
          ? t("llmDot.partial")
          : t("llmDot.missing");

  const color =
    state === "pending"
      ? "bg-subtle"
      : state === "ready"
        ? "bg-green-500"
        : state === "partial"
          ? "bg-severity-medium"
          : "bg-subtle";

  return (
    <Tooltip text={tip} className="flex items-center" as="div">
      <span
        role="img"
        aria-label={tip}
        className={cn(
          "inline-block h-2 w-2 rounded-full",
          color,
          data?.server_running && "animate-pulse [animation-duration:2.5s]",
        )}
      />
    </Tooltip>
  );
}

/** Locale switcher EN | VI. */
function LocaleSwitcher() {
  const { i18n } = useTranslation();
  const current = i18n.language.startsWith("vi") ? "vi" : "en";
  return (
    <div
      role="group"
      aria-label="Language / Ngôn ngữ"
      className="flex overflow-hidden rounded-lg border border-line"
    >
      {(["en", "vi"] as const).map((lng) => (
        <button
          key={lng}
          type="button"
          aria-pressed={current === lng}
          onClick={() => {
            void i18n.changeLanguage(lng);
            document.documentElement.lang = lng;
            try {
              window.localStorage.setItem("difflens.locale", lng);
            } catch {
              // Non-fatal.
            }
          }}
          className={cn(
            "px-2 py-1 text-xs font-semibold uppercase transition-colors",
            current === lng
              ? "bg-accent-soft text-accent"
              : "bg-surface text-muted hover:bg-surface-2 hover:text-fg",
          )}
        >
          {lng}
        </button>
      ))}
    </div>
  );
}

const NEXT_THEME: Record<ThemeMode, ThemeMode> = {
  system: "light",
  light: "dark",
  dark: "system",
};

const THEME_TIP_KEY: Record<ThemeMode, string> = {
  system: "settings.theme.system",
  light: "settings.theme.light",
  dark: "settings.theme.dark",
};

/** Theme indicator — click cycles System → Light → Dark. */
function ThemeButton() {
  const { t } = useTranslation();
  const { mode, setMode } = useTheme();
  const Icon = mode === "system" ? Monitor : mode === "light" ? Sun : Moon;
  return (
    <Tooltip text={t(THEME_TIP_KEY[mode])} below className="flex" as="div">
      <button
        type="button"
        aria-label={`${t("settings.theme.label")}: ${t(THEME_TIP_KEY[mode])}`}
        onClick={() => setMode(NEXT_THEME[mode])}
        className="flex h-8 w-8 items-center justify-center rounded-lg text-muted hover:bg-surface-2 hover:text-fg"
      >
        <Icon aria-hidden className="h-4 w-4" />
      </button>
    </Tooltip>
  );
}

/** Browser-dev banner: honest about missing runtime capabilities. */
function TauriBanner() {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(false);
  const [needed, setNeeded] = useState(false);
  useEffect(() => {
    setNeeded(!isTauri());
  }, []);
  if (!needed || dismissed) return null;
  return (
    <div
      role="status"
      className="flex items-start gap-3 border-b border-severity-medium-edge bg-severity-medium-soft px-4 py-2 text-severity-medium-text"
    >
      <div className="min-w-0 flex-1">
        <p className="text-xs font-semibold">{t("banner.browserTitle")}</p>
        <p className="text-xs opacity-90">{t("banner.browserBody")}</p>
      </div>
      <button
        type="button"
        aria-label={t("banner.dismiss")}
        onClick={() => setDismissed(true)}
        className="rounded-md p-1 hover:bg-severity-medium-edge/50"
      >
        <X aria-hidden className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------

export function AppShell() {
  return (
    <div className="flex h-screen flex-col overflow-hidden bg-bg text-fg">
      <TauriBanner />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-11 shrink-0 items-center justify-end gap-2 border-b border-line bg-surface px-3">
            <LlmIndicator />
            <div className="mx-1 h-4 w-px bg-line" aria-hidden />
            <LocaleSwitcher />
            <ThemeButton />
          </header>
          <main className="min-h-0 flex-1 overflow-hidden">
            <Outlet />
          </main>
        </div>
      </div>
    </div>
  );
}
