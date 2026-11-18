//! Diff page header: file names, format, stat chips, embeddings indicator,
//! live progress while the job runs.

import { useTranslation } from "react-i18next";
import { ArrowLeftRight, CircleCheck, CircleX, Square, Zap } from "lucide-react";

import { FormatBadge, StatChip } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Tooltip } from "@/components/ui/tooltip";
import { ExportMenu } from "@/components/diff/ExportMenu";
import { fileBaseName } from "@/lib/format";
import { cn } from "@/lib/cn";
import type { JobSnapshot } from "@/lib/types";

const STAGE_KEYS = new Set([
  "queued",
  "extract",
  "ocr",
  "align",
  "diff",
  "enrich",
  "done",
  "failed",
  "cancelled",
  "running",
]);

export function DiffHeader({
  snapshot,
  onCancel,
}: {
  snapshot: JobSnapshot;
  onCancel?: () => void;
}) {
  const { t } = useTranslation();
  const result = snapshot.result;
  const running = snapshot.state === "running";

  const stageLabel = STAGE_KEYS.has(snapshot.stage)
    ? t(`stage.${snapshot.stage}`)
    : snapshot.stage;

  return (
    <header className="flex shrink-0 flex-col gap-2 border-b border-line bg-surface px-4 py-2.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <h1 className="flex min-w-0 items-center gap-2 text-sm font-semibold text-fg">
          <span className="truncate" title={result ? result.old_path : undefined}>
            {result ? fileBaseName(result.old_path) : snapshot.job_id}
          </span>
          <ArrowLeftRight aria-hidden className="h-3.5 w-3.5 shrink-0 text-subtle" />
          <span className="truncate" title={result ? result.new_path : undefined}>
            {result ? fileBaseName(result.new_path) : ""}
          </span>
          {result ? <FormatBadge format={result.format} /> : null}
        </h1>

        {result ? (
          <div className="tabular flex flex-wrap items-center gap-1.5" aria-label={t("diff.title")}>
            <StatChip
              glyph="+"
              count={result.stats.added}
              label={t("diff.stats.added")}
              tone="insert"
            />
            <StatChip
              glyph="−"
              count={result.stats.removed}
              label={t("diff.stats.removed")}
              tone="delete"
            />
            <StatChip
              glyph="~"
              count={result.stats.modified}
              label={t("diff.stats.modified")}
              tone="modified"
            />
            <StatChip
              glyph="⇄"
              count={result.stats.moved}
              label={t("diff.stats.moved")}
              tone="moved"
            />
            <StatChip
              glyph="="
              count={result.stats.equal}
              label={t("diff.stats.equal")}
              tone="neutral"
            />
          </div>
        ) : null}

        {result ? (
          <Tooltip
            text={t("diff.embeddingsTip")}
            below
            className="flex items-center"
            as="div"
          >
            <span
              className={cn(
                "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs",
                result.embeddings_used
                  ? "border-accent-edge bg-accent-soft text-accent"
                  : "border-line bg-surface-2 text-muted",
              )}
            >
              <Zap aria-hidden className="h-3 w-3" />
              {result.embeddings_used ? t("diff.embeddingsOn") : t("diff.embeddingsOff")}
            </span>
          </Tooltip>
        ) : null}

        <div className="ms-auto flex items-center gap-2">
          {/* Phase-08: report export for the finished (or hydrated session) view. */}
          {snapshot.state === "done" && result ? (
            <ExportMenu targetId={snapshot.job_id} />
          ) : null}
          {snapshot.state === "done" ? (
            <span className="flex items-center gap-1 text-xs text-insert-fg">
              <CircleCheck aria-hidden className="h-3.5 w-3.5" />
              {t("jobState.done")}
            </span>
          ) : null}
          {snapshot.state === "failed" ? (
            <span className="flex items-center gap-1 text-xs text-delete-fg">
              <CircleX aria-hidden className="h-3.5 w-3.5" />
              {t("jobState.failed")}
            </span>
          ) : null}
          {running && onCancel ? (
            <Button variant="ghost" size="sm" onClick={onCancel}>
              <Square aria-hidden className="h-3 w-3" />
              {t("common.cancel")}
            </Button>
          ) : null}
        </div>
      </div>

      {running ? (
        <div className="flex items-center gap-3">
          <Progress
            value={snapshot.percent}
            indeterminate={snapshot.percent === 0}
            label={`${stageLabel} — ${snapshot.percent}%`}
            className="max-w-md"
          />
          <span className="tabular text-xs text-muted">
            {t("diff.running")} · {stageLabel} · {snapshot.percent}%
          </span>
        </div>
      ) : null}

      {snapshot.state === "failed" && snapshot.error ? (
        <p className="font-mono text-xs text-delete-fg" role="alert">
          {snapshot.error}
        </p>
      ) : null}
      {snapshot.state === "cancelled" ? (
        <p className="text-xs text-muted" role="status">
          {t("diff.cancelled")}
        </p>
      ) : null}
    </header>
  );
}
