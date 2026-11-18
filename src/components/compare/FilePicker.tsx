//! One file slot of the compare wizard: browse button, drag-drop visual
//! target, format auto-detect badge, clear action.

import { useTranslation } from "react-i18next";
import { FileText, FolderOpen, TriangleAlert, X } from "lucide-react";

import { FormatBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardBody } from "@/components/ui/card";
import { fileBaseName, formatFromExtension } from "@/lib/format";
import { cn } from "@/lib/cn";

export function FilePicker({
  side,
  label,
  path,
  busy,
  onBrowse,
  onClear,
  invalid,
}: {
  side: "old" | "new";
  label: string;
  path: string | null;
  busy: boolean;
  onBrowse: () => void;
  onClear: () => void;
  /** Path set but extension unsupported. */
  invalid: boolean;
}) {
  const { t } = useTranslation();
  const format = path ? formatFromExtension(path) : null;
  const empty = path === null;

  return (
    <Card
      className={cn(
        "flex-1 transition-colors",
        invalid && "border-severity-medium-edge",
      )}
      data-testid={`picker-${side}`}
    >
      <CardBody className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold tracking-wide text-subtle uppercase">
            {label}
          </span>
          {format ? <FormatBadge format={format} /> : null}
        </div>

        {empty ? (
          <button
            type="button"
            onClick={onBrowse}
            disabled={busy}
            className={cn(
              "flex h-24 w-full flex-col items-center justify-center gap-1.5 rounded-lg border-2 border-dashed border-line-strong text-muted transition-colors hover:border-accent hover:bg-accent-soft/40 hover:text-accent",
              busy && "cursor-wait opacity-70",
            )}
          >
            <FolderOpen aria-hidden className="h-5 w-5" />
            <span className="text-xs font-medium">{t("wizard.dropHint")}</span>
            <span className="text-xs opacity-70">{t("wizard.choose")}</span>
          </button>
        ) : (
          <div
            className={cn(
              "flex h-24 w-full flex-col justify-center gap-2 rounded-lg border border-line bg-surface-2 px-3",
              invalid && "border-severity-medium-edge bg-severity-medium-soft/60",
            )}
          >
            <div className="flex items-center gap-2">
              <FileText aria-hidden className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1 truncate text-sm font-medium text-fg" title={path}>
                {fileBaseName(path)}
              </span>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`${t("wizard.clear")} — ${label}`}
                onClick={onClear}
              >
                <X aria-hidden className="h-3.5 w-3.5" />
              </Button>
            </div>
            <p className="truncate text-xs text-subtle" title={path}>
              {path}
            </p>
            {invalid ? (
              <p className="flex items-center gap-1.5 text-xs text-severity-medium-text">
                <TriangleAlert aria-hidden className="h-3.5 w-3.5 shrink-0" />
                {t("wizard.unsupported")}
              </p>
            ) : null}
          </div>
        )}
      </CardBody>
    </Card>
  );
}
