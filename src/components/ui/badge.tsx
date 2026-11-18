//! Badges: neutral chip + severity badge + format badge + kind chip.
//! Severity colors come from the @theme severity palette (AA in both themes).

import type { HTMLAttributes, ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/cn";
import { severityStyle } from "@/lib/format";
import type { DocFormat, Severity } from "@/lib/types";

export function Badge({
  className,
  tone = "neutral",
  ...rest
}: HTMLAttributes<HTMLSpanElement> & {
  tone?: "neutral" | "accent" | "success" | "warning" | "danger";
}) {
  const tones: Record<string, string> = {
    neutral: "bg-surface-2 text-muted border-line",
    accent: "bg-accent-soft text-accent border-accent-edge",
    success: "bg-insert-bg text-insert-fg border-transparent",
    warning: "bg-severity-medium-soft text-severity-medium-text border-severity-medium-edge",
    danger: "bg-delete-bg text-delete-fg border-transparent",
  };
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
        tones[tone],
        className,
      )}
      {...rest}
    />
  );
}

/** Localized severity badge ("High", "Nghiêm trọng", …). */
export function SeverityBadge({ severity, className }: { severity: Severity; className?: string }) {
  const { t } = useTranslation();
  const style = severityStyle(severity);
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
        style.bg,
        style.text,
        style.border,
        className,
      )}
    >
      <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", style.dot)} />
      {t(`severity.${severity}`)}
    </span>
  );
}

/** Document format badge (PDF/DOCX/…). */
export function FormatBadge({ format, className }: { format: DocFormat; className?: string }) {
  const { t } = useTranslation();
  return (
    <Badge tone="accent" className={cn("uppercase tabular", className)}>
      {t(`format.${format}`)}
    </Badge>
  );
}

/** Signed-count stat chip used in the diff header (+3 modified …). */
export function StatChip({
  glyph,
  count,
  label,
  tone,
}: {
  glyph: string;
  count: number;
  label: string;
  tone: "insert" | "delete" | "modified" | "moved" | "neutral";
}) {
  const tones: Record<string, string> = {
    insert: "text-insert-fg bg-insert-row",
    delete: "text-delete-fg bg-delete-row",
    modified: "text-severity-medium-text bg-severity-medium-soft",
    moved: "text-accent bg-accent-soft",
    neutral: "text-muted bg-surface-2",
  };
  return (
    <span
      className={cn(
        "tabular inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium",
        tones[tone],
      )}
      title={label}
    >
      <span aria-hidden className="font-semibold">
        {glyph}
      </span>
      {count}
      <span className="font-normal opacity-80">{label}</span>
    </span>
  );
}

/** Small labelled section header inside cards/pages. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <div className="text-xs font-semibold tracking-wide text-subtle uppercase">{children}</div>
  );
}
