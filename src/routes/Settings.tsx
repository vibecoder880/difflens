//! Settings page: language, theme, OCR preference, embedding note, about.

import type { ChangeEvent } from "react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Bot, Globe, Info, Moon, ScanText, Sun } from "lucide-react";

import { Card, CardBody, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import pkg from "../../package.json";
import { LOCALES, type Locale } from "@/lib/i18n";
import { loadOcrPref, persistOcrPref } from "@/lib/prefs";
import { useTheme, THEME_MODES, type ThemeMode } from "@/lib/theme";

// ---------------------------------------------------------------------------

export function SettingsPage() {
  const { t, i18n } = useTranslation();
  const { mode, setMode } = useTheme();
  const [ocrPref, setOcrPref] = useState(loadOcrPref);

  const onLocaleChange = (e: ChangeEvent<HTMLSelectElement>) => {
    const next = e.target.value;
    void i18n.changeLanguage(next);
    document.documentElement.lang = next;
    try {
      window.localStorage.setItem("difflens.locale", next);
    } catch {
      // Non-fatal.
    }
  };

  const onThemeChange = (e: ChangeEvent<HTMLSelectElement>) => {
    const next = e.target.value;
    if ((THEME_MODES as readonly string[]).includes(next)) setMode(next as ThemeMode);
  };

  const onOcrChange = (next: boolean) => {
    setOcrPref(next);
    persistOcrPref(next);
  };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-2xl flex-col gap-4 px-6 py-6">
        <header>
          <h1 className="text-lg font-semibold tracking-tight text-fg">{t("settings.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("settings.subtitle")}</p>
        </header>

        {/* Appearance: theme */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Moon aria-hidden className="h-4 w-4 text-accent" />
              {t("settings.appearance")}
            </CardTitle>
          </CardHeader>
          <CardBody>
            <label htmlFor="theme-select" className="mb-1 block text-xs font-medium text-muted">
              {t("settings.theme.label")}
            </label>
            <Select id="theme-select" value={mode} onChange={onThemeChange} className="max-w-52">
              {THEME_MODES.map((m) => (
                <option key={m} value={m}>
                  {t(`settings.theme.${m}`)}
                </option>
              ))}
            </Select>
          </CardBody>
        </Card>

        {/* Language */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Globe aria-hidden className="h-4 w-4 text-accent" />
              {t("settings.language.title")}
            </CardTitle>
            <CardDescription>{t("settings.language.desc")}</CardDescription>
          </CardHeader>
          <CardBody>
            <label htmlFor="locale-select" className="mb-1 block text-xs font-medium text-muted">
              {t("settings.language.title")}
            </label>
            <Select
              id="locale-select"
              value={i18n.language.startsWith("vi") ? "vi" : "en"}
              onChange={onLocaleChange}
              className="max-w-52"
            >
              {LOCALES.map((lng: Locale) => (
                <option key={lng} value={lng}>
                  {lng === "en" ? "English" : "Tiếng Việt"}
                </option>
              ))}
            </Select>
          </CardBody>
        </Card>

        {/* OCR preference */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <ScanText aria-hidden className="h-4 w-4 text-accent" />
              {t("settings.ocr.title")}
            </CardTitle>
            <CardDescription>{t("settings.ocr.desc")}</CardDescription>
          </CardHeader>
          <CardBody className="flex items-center justify-between gap-4">
            <span className="text-sm text-fg">{t("settings.ocr.enable")}</span>
            <Switch checked={ocrPref} onChange={onOcrChange} label={t("settings.ocr.enable")} />
          </CardBody>
        </Card>

        {/* Embedding model note */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Sun aria-hidden className="h-4 w-4 text-accent" />
              {t("settings.embedding.title")}
            </CardTitle>
          </CardHeader>
          <CardBody className="space-y-1.5 text-xs text-muted">
            <p>{t("settings.embedding.bundled")}</p>
            <p>{t("settings.embedding.comingSoon")}</p>
          </CardBody>
        </Card>

        {/* About */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Info aria-hidden className="h-4 w-4 text-accent" />
              {t("settings.about.title")}
            </CardTitle>
          </CardHeader>
          <CardBody className="space-y-1.5 text-xs text-muted">
            <p className="flex items-center gap-2">
              <Bot aria-hidden className="h-3.5 w-3.5" />
              <span className="font-semibold text-fg">DiffLens</span>
              <span className="tabular">{t("settings.about.version", { version: pkg.version })}</span>
            </p>
            <p>{t("settings.about.license")}</p>
            <p>{t("settings.about.thirdParty")}</p>
            <p className="font-mono text-[11px] text-subtle">{t("settings.about.repo")}</p>
          </CardBody>
        </Card>
      </div>
    </div>
  );
}
