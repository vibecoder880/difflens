//! Live progress card for the most recent diff job (diff://progress +
//! get_diff polling). Shown on the wizard while a job runs.

import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ArrowRight, Loader2, Square } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardBody } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { useLastJob } from "@/lib/jobStore";
import { cancelDiff, getDiff } from "@/lib/tauri";

const KNOWN_STAGES = new Set([
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

function stageLabel(stage: string): string {
  const { t } = useTranslation();
  return KNOWN_STAGES.has(stage) ? t(`stage.${stage}`) : stage;
}

export function JobProgressCard({ className }: { className?: string }) {
  const { t } = useTranslation();
  const jobId = useLastJob();
  const { data, isPending, refetch, error } = useQuery({
    queryKey: ["diff", jobId],
    queryFn: () => getDiff(jobId ?? ""),
    enabled: jobId !== null,
    refetchInterval: (query) =>
      query.state.data && query.state.data.state === "running" ? 700 : false,
  });

  if (jobId === null) {
    return (
      <p className={className ?? ""}>
        <span className="text-xs text-subtle">{t("wizard.recent.empty")}</span>
      </p>
    );
  }
  if (isPending) {
    return (
      <Card className={className}>
        <CardBody className="space-y-2">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-1.5 w-full" />
        </CardBody>
      </Card>
    );
  }
  if (error || !data) {
    return (
      <p className={className ?? ""}>
        <span className="text-xs text-delete-fg">{t("diff.loadError")}</span>
      </p>
    );
  }

  const running = data.state === "running";
  return (
    <Card className={className}>
      <CardBody className="space-y-2.5">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            {running ? (
              <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin text-accent" />
            ) : null}
            <span className="text-xs font-semibold text-fg">{t("wizard.recent.title")}</span>
            <Badge
              tone={
                data.state === "done"
                  ? "success"
                  : data.state === "failed"
                    ? "danger"
                    : data.state === "cancelled"
                      ? "warning"
                      : "accent"
              }
            >
              {t(`jobState.${data.state}`)}
            </Badge>
          </div>
          <div className="flex items-center gap-1.5">
            {running ? (
              <Button
                variant="ghost"
                size="sm"
                aria-label={t("wizard.recent.cancel")}
                onClick={() => void cancelDiff(jobId).then(() => refetch())}
              >
                <Square aria-hidden className="h-3 w-3" />
                {t("wizard.recent.cancel")}
              </Button>
            ) : null}
            {data.state !== "running" ? (
              <Link
                to="/diff/$jobId"
                params={{ jobId }}
                className="inline-flex h-8 items-center gap-1.5 rounded-lg px-3 text-xs font-medium text-accent hover:bg-accent-soft"
              >
                {t("wizard.recent.open")}
                <ArrowRight aria-hidden className="h-3.5 w-3.5" />
              </Link>
            ) : null}
          </div>
        </div>
        <Progress
          value={data.percent}
          indeterminate={running && data.percent === 0}
          label={`${stageLabel(data.stage)} — ${data.percent}%`}
        />
        <p className="tabular text-xs text-muted">
          {stageLabel(data.stage)} · {data.percent}%
        </p>
      </CardBody>
    </Card>
  );
}
