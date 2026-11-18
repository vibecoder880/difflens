//! Downloads page: llama-server install card, model cards with verified
//! download progress + cancel, and the honest OCR availability card.

import { useCallback, useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  BadgeCheck,
  CircleCheck,
  CircleX,
  Cpu,
  HardDriveDownload,
  Loader2,
  ScanText,
  Server,
  ShieldCheck,
} from "lucide-react";

import { ErrorCard } from "@/components/ErrorCard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardBody, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip } from "@/components/ui/tooltip";
import { formatBytes } from "@/lib/format";
import { upsertDownload, useDownloadJob } from "@/lib/downloadStore";
import {
  llmCancel,
  llmInstallServer,
  llmDownloadModel,
  llmStatus,
  ocrStatus,
  onLlmDownload,
} from "@/lib/tauri";
import type { LlmModelStatus } from "@/lib/types";

/** Friendly display names for manifest model ids (fallback: raw id). */
const MODEL_NAMES: Record<string, string> = {
  "qwen3-4b-instruct-2507-q4_k_m": "Qwen3 4B Instruct",
  "qwen2.5-3b-instruct-q4_k_m": "Qwen2.5 3B Instruct",
  "qwen3-0.6b-q8_0-test": "Qwen3 0.6B (test)",
};

// ---------------------------------------------------------------------------
// Shared download progress row (server + model cards)
// ---------------------------------------------------------------------------

function DownloadProgress({
  jobId,
  onFinished,
}: {
  jobId: string;
  /** Fired when the payload reaches a terminal state (failed/cancelled). */
  onFinished?: () => void;
}) {
  const { t } = useTranslation();
  const job = useDownloadJob(jobId);

  useEffect(() => {
    if (job !== null && (job.state === "failed" || job.state === "cancelled")) onFinished?.();
  }, [job, onFinished]);

  if (job === null) return null;
  if (job.state === "done") return null;
  if (job.state === "failed") {
    return (
      <p className="flex flex-wrap items-center gap-1.5 text-xs text-delete-fg" role="alert">
        <CircleX aria-hidden className="h-3.5 w-3.5 shrink-0" />
        {t("downloads.progress.failed")}
        {job.error ? <span className="font-mono break-all">{job.error}</span> : null}
      </p>
    );
  }
  if (job.state === "cancelled") {
    return (
      <p role="status" className="text-xs text-muted">
        {t("downloads.progress.cancelled")}
      </p>
    );
  }
  return (
    <div className="flex items-center gap-2">
      <Progress value={job.percent} label={`${job.percent}%`} className="flex-1" />
      <span className="tabular shrink-0 text-xs text-muted">
        {job.percent}% · {formatBytes(job.bytes_done)}
        {job.bytes_total !== null ? ` / ${formatBytes(job.bytes_total)}` : ""}
      </span>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void llmCancel(job.job_id)}
        aria-label={t("downloads.model.cancel")}
      >
        {t("common.cancel")}
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Server card
// ---------------------------------------------------------------------------

function ServerCard() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [installClicked, setInstallClicked] = useState(false);
  const status = useQuery({ queryKey: ["llm", "status"], queryFn: llmStatus, retry: 0 });
  const serverJobId = status.data ? `server-${status.data.server_tag}` : null;
  const job = useDownloadJob(serverJobId);

  const install = () => {
    setInstallClicked(true);
    llmInstallServer().catch(() => {
      setInstallClicked(false);
    });
  };

  if (status.isPending) {
    return (
      <Card>
        <CardBody className="space-y-2">
          <Skeleton className="h-4 w-48" />
          <Skeleton className="h-3 w-72" />
        </CardBody>
      </Card>
    );
  }
  if (status.isError || !status.data) {
    return (
      <ErrorCard
        onRetry={() => void queryClient.invalidateQueries({ queryKey: ["llm", "status"] })}
      />
    );
  }

  const data = status.data;
  const installed = data.server_installed;
  const running = serverJobId !== null && job?.state === "running";
  const refresh = () => queryClient.invalidateQueries({ queryKey: ["llm", "status"] });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          <Server aria-hidden className="h-4 w-4 text-accent" />
          {t("downloads.server.title")}
          {installed ? (
            <Badge tone="success">
              <CircleCheck aria-hidden className="h-3 w-3" />
              {t("downloads.server.installed")}
            </Badge>
          ) : (
            <Badge tone="neutral">{t("downloads.server.missing")}</Badge>
          )}
          {data.server_running ? <Badge tone="accent">{t("downloads.server.running")}</Badge> : null}
        </CardTitle>
        <CardDescription>{t("downloads.server.desc")}</CardDescription>
      </CardHeader>
      <CardBody className="flex flex-wrap items-center gap-2">
        <span className="tabular text-xs text-muted">
          {t("downloads.server.versionTag", { tag: data.server_tag })}
        </span>
        {data.server_on_path && !installed ? (
          <Badge tone="neutral">{t("downloads.server.onPath")}</Badge>
        ) : null}
        {!installed ? (
          <Button
            variant="primary"
            size="sm"
            busy={running || (installClicked && job === null)}
            onClick={install}
          >
            <HardDriveDownload aria-hidden className="h-4 w-4" />
            {running ? t("downloads.server.installing") : t("downloads.server.install")}
          </Button>
        ) : null}
        {serverJobId !== null && running ? (
          <div className="mt-1 w-full">
            <DownloadProgress jobId={serverJobId} onFinished={refresh} />
          </div>
        ) : null}
      </CardBody>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Model cards
// ---------------------------------------------------------------------------

function ModelCard({ model, onChanged }: { model: LlmModelStatus; onChanged: () => void }) {
  const { t } = useTranslation();
  const jobId = `model-${model.id}`;
  const job = useDownloadJob(jobId);
  const [clicked, setClicked] = useState(false);
  const running = job?.state === "running";

  // The in-flight window between command resolution and the first event.
  useEffect(() => {
    if (job !== null) setClicked(false);
  }, [job]);

  const download = () => {
    setClicked(true);
    llmDownloadModel(model.id).catch(() => {
      setClicked(false);
    });
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          <Cpu aria-hidden className="h-4 w-4 text-accent" />
          {MODEL_NAMES[model.id] ?? model.id}
          <Badge tone={model.role === "primary" ? "accent" : "neutral"}>
            {t(`downloads.model.role.${model.role}`)}
          </Badge>
          {model.present ? (
            <Badge tone="success">
              <CircleCheck aria-hidden className="h-3 w-3" />
              {t("downloads.model.downloaded")}
            </Badge>
          ) : null}
        </CardTitle>
        <CardDescription>{t(`downloads.model.description.${model.id}`)}</CardDescription>
      </CardHeader>
      <CardBody className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
          <span className="tabular">
            {t("downloads.model.size", {
              size: formatBytes(model.bytes),
              license: model.license,
            })}
          </span>
          {model.verified ? (
            <Tooltip text={t("downloads.model.shaVerified")} below as="span" className="flex">
              <Badge tone="success">
                <ShieldCheck aria-hidden className="h-3 w-3" />
                {t("downloads.model.shaVerified")}
              </Badge>
            </Tooltip>
          ) : (
            <Badge tone="warning">{t("downloads.model.unverified")}</Badge>
          )}
        </div>

        {running ? (
          <DownloadProgress jobId={jobId} onFinished={onChanged} />
        ) : model.present ? (
          <p className="flex items-center gap-1.5 text-xs text-insert-fg">
            <BadgeCheck aria-hidden className="h-3.5 w-3.5" />
            {t("downloads.model.present")} · {formatBytes(model.downloaded_bytes)}
          </p>
        ) : (
          <div>
            <Button variant="primary" size="sm" busy={clicked} onClick={download}>
              <HardDriveDownload aria-hidden className="h-4 w-4" />
              {t("downloads.model.download")}
            </Button>
          </div>
        )}
      </CardBody>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// OCR card (honest availability + static install copy)
// ---------------------------------------------------------------------------

function OcrCard() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["ocr", "status"],
    queryFn: ocrStatus,
    retry: 0,
    staleTime: 60_000,
  });

  if (query.isPending) {
    return (
      <Card>
        <CardBody className="flex items-center gap-2 text-xs text-muted">
          <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
          {t("downloads.ocr.checking")}
        </CardBody>
      </Card>
    );
  }
  const data = query.data;
  if (!data) {
    return (
      <ErrorCard
        onRetry={() => void queryClient.invalidateQueries({ queryKey: ["ocr", "status"] })}
      />
    );
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          <ScanText aria-hidden className="h-4 w-4 text-accent" />
          {t("downloads.ocr.title")}
          {data.available ? (
            <Badge tone="success">
              <CircleCheck aria-hidden className="h-3 w-3" />
              {t("downloads.ocr.available")}
            </Badge>
          ) : (
            <Badge tone="warning">{t("downloads.ocr.missing")}</Badge>
          )}
        </CardTitle>
        <CardDescription>{t("downloads.ocr.installHint")}</CardDescription>
      </CardHeader>
      <CardBody className="space-y-1 text-xs text-muted">
        {data.binary ? (
          <p className="font-mono break-all">{t("downloads.ocr.binary", { path: data.binary })}</p>
        ) : null}
        {data.langs.length > 0 ? (
          <p className="tabular">{t("downloads.ocr.langs", { langs: data.langs.join(", ") })}</p>
        ) : null}
        {!data.available ? (
          <p className="text-xs text-muted">
            {data.has_vie
              ? t("downloads.ocr.missing")
              : `${t("downloads.ocr.missing")} — tesseract-ocr-vie`}
          </p>
        ) : null}
      </CardBody>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export function DownloadsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const status = useQuery({
    queryKey: ["llm", "status"],
    queryFn: llmStatus,
    retry: 0,
    refetchInterval: 30_000,
  });

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ["llm", "status"] });
  }, [queryClient]);

  // Single event subscription for the whole page (unlisten on unmount).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void onLlmDownload((payload) => {
      upsertDownload(payload);
      if (payload.state === "done") refresh();
    })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

  const models = status.data?.models ?? [];

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-6 py-6">
        <header>
          <h1 className="text-lg font-semibold tracking-tight text-fg">{t("downloads.title")}</h1>
          <p className="mt-0.5 text-sm text-muted">{t("downloads.subtitle")}</p>
        </header>

        <ServerCard />

        {status.isPending ? (
          <Skeleton className="h-40 w-full" />
        ) : status.isError ? (
          <ErrorCard
            onRetry={() => void queryClient.invalidateQueries({ queryKey: ["llm", "status"] })}
          />
        ) : (
          models.map((model) => <ModelCard key={model.id} model={model} onChanged={refresh} />)
        )}

        <OcrCard />
      </div>
    </div>
  );
}
