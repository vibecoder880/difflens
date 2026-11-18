//! Export ▾ menu in the diff header (phase-08): HTML / Markdown report,
//! saved through the native save dialog; the Rust command renders and
//! writes the file (plugin-fs is not installed).

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, CircleCheck, CircleX, FileCode2, FileText, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { exportReport, pickReportSavePath } from "@/lib/tauri";
import type { ReportFormat } from "@/lib/types";

type ExportState = "idle" | "saving" | "saved" | "failed";

export function ExportMenu({ targetId }: { targetId: string }) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<ExportState>("idle");
  const rootRef = useRef<HTMLDivElement>(null);

  // Close on outside click / Esc.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current !== null && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const run = async (format: ReportFormat) => {
    setOpen(false);
    setState("saving");
    try {
      const path = await pickReportSavePath(targetId, format);
      if (path === null) {
        setState("idle"); // cancelled
        return;
      }
      await exportReport(targetId, format, path, i18n.language);
      setState("saved");
    } catch {
      setState("failed");
    }
  };

  const itemClass =
    "flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-fg hover:bg-surface-2";

  return (
    <div ref={rootRef} className="relative flex items-center gap-1">
      <Button
        variant="secondary"
        size="sm"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {t("export.menu")}
        <ChevronDown aria-hidden className="h-3 w-3" />
      </Button>
      {open ? (
        <div
          role="menu"
          aria-label={t("export.menu")}
          className="absolute top-full right-0 z-20 mt-1 w-48 rounded-lg border border-line bg-surface py-1 shadow-lg"
        >
          <button
            type="button"
            role="menuitem"
            className={itemClass}
            onClick={() => void run("html")}
          >
            <FileCode2 aria-hidden className="h-3.5 w-3.5 text-accent" />
            {t("export.html")}
          </button>
          <button
            type="button"
            role="menuitem"
            className={itemClass}
            onClick={() => void run("md")}
          >
            <FileText aria-hidden className="h-3.5 w-3.5 text-accent" />
            {t("export.markdown")}
          </button>
        </div>
      ) : null}
      {state === "saving" ? (
        <span className="flex items-center gap-1 text-xs text-muted" role="status">
          <Loader2 aria-hidden className="h-3 w-3 animate-spin" />
          {t("export.saving")}
        </span>
      ) : null}
      {state === "saved" ? (
        <span className="flex items-center gap-1 text-xs text-insert-fg" role="status">
          <CircleCheck aria-hidden className="h-3 w-3" />
          {t("export.saved")}
        </span>
      ) : null}
      {state === "failed" ? (
        <span className="flex items-center gap-1 text-xs text-delete-fg" role="alert">
          <CircleX aria-hidden className="h-3 w-3" />
          {t("export.failed")}
        </span>
      ) : null}
    </div>
  );
}
