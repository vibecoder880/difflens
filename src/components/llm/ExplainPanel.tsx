//! LLM explanation panel: streams explain_change tokens into a plain-text
//! panel with cancel; honest setup empty-state when no local model exists.

import { useEffect, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Bot, Download, ShieldAlert, Square } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import {
  TauriRequiredError,
  explainChange,
  llmCancel,
  llmStatus,
  onLlmDone,
  onLlmToken,
} from "@/lib/tauri";
import type { LlmDonePayload } from "@/lib/types";

interface ExplainState {
  /** The `explain-<n>` job id — non-null while a stream is active/finished. */
  jobId: string | null;
  text: string;
  streaming: boolean;
  done: LlmDonePayload | null;
  error: string | null;
}

const IDLE: ExplainState = { jobId: null, text: "", streaming: false, done: null, error: null };

export function ExplainPanel({ diffJobId, changeId }: { diffJobId: string; changeId: string }) {
  const { t, i18n } = useTranslation();
  const [state, setState] = useState<ExplainState>(IDLE);
  const streamRef = useRef<HTMLDivElement>(null);

  const status = useQuery({
    queryKey: ["llm", "status"],
    queryFn: llmStatus,
    retry: 0,
    staleTime: 10_000,
  });

  const serverOk =
    (status.data?.server_installed || status.data?.server_on_path) ?? false;
  const anyModel = status.data?.models.some((m) => m.present) ?? false;
  const ready = serverOk && anyModel;

  // Subscribe to stream events while a job is active (unlisten on cleanup).
  useEffect(() => {
    if (state.jobId === null) return;
    const jobId = state.jobId;
    let active = true;
    const unsubscribes: Array<() => void> = [];
    void onLlmToken((payload) => {
      if (!active || payload.job_id !== jobId) return;
      setState((prev) => ({ ...prev, text: prev.text + payload.delta }));
    })
      .then((u) => unsubscribes.push(u))
      .catch(() => undefined);
    void onLlmDone((payload) => {
      if (!active || payload.job_id !== jobId) return;
      setState((prev) => ({
        ...prev,
        streaming: false,
        done: payload,
        error: payload.ok ? null : (payload.error ?? t("explain.errorTitle")),
      }));
    })
      .then((u) => unsubscribes.push(u))
      .catch(() => undefined);
    return () => {
      active = false;
      for (const u of unsubscribes) u();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.jobId]);

  // Scroll-locked: keep the newest token visible while streaming.
  useEffect(() => {
    const el = streamRef.current;
    if (el && state.streaming) el.scrollTop = el.scrollHeight;
  }, [state.text, state.streaming]);

  const start = () => {
    setState({ ...IDLE, streaming: true });
    explainChange(diffJobId, changeId, i18n.language.slice(0, 2))
      .then((jobId) => setState((prev) => ({ ...prev, jobId })))
      .catch((e) =>
        setState({
          ...IDLE,
          error:
            e instanceof TauriRequiredError
              ? e.message
              : String(e).replace(/^Error\b[: ]*/i, ""),
        }),
      );
  };

  const stop = () => {
    if (state.jobId !== null) void llmCancel(state.jobId);
    setState((prev) => ({ ...prev, streaming: false }));
  };

  // ------------------------------------------------------------------
  // Setup empty-state (honest: requires local model + server)
  // ------------------------------------------------------------------
  if (!ready) {
    return (
      <div className="rounded-lg border border-line bg-surface-2">
        <EmptyState
          icon={Download}
          title={t("explain.needsSetup")}
          body={t("explain.needsSetupBody")}
          action={
            <Link
              to="/downloads"
              className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-accent px-3 text-xs font-medium text-accent-fg hover:bg-accent-hover"
            >
              {t("explain.goDownloads")}
            </Link>
          }
          className="py-8"
        />
      </div>
    );
  }

  const done = state.done;
  const seconds = done ? Math.max(1, Math.round(Number(done.elapsed_ms) / 1000)) : null;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Button
            variant={state.streaming ? "secondary" : "primary"}
            size="sm"
            busy={state.streaming}
            onClick={state.streaming ? stop : start}
          >
            {state.streaming ? (
              <>
                <Square aria-hidden className="h-3 w-3" />
                {t("explain.stop")}
              </>
            ) : state.jobId !== null ? (
              t("explain.again")
            ) : (
              <>
                <Bot aria-hidden className="h-4 w-4" />
                {t("explain.button")}
              </>
            )}
          </Button>
          {state.streaming ? (
            <span className="text-xs text-muted">{t("explain.streaming")}</span>
          ) : null}
          {done && done.ok && seconds !== null ? (
            <span className="tabular text-xs text-subtle">{t("explain.doneIn", { seconds })}</span>
          ) : null}
        </div>
        <Badge tone="warning" className="shrink-0">
          <ShieldAlert aria-hidden className="h-3 w-3" />
          {t("explain.disclaimer")}
        </Badge>
      </div>

      {state.error ? (
        <p role="alert" className="font-mono text-xs break-words text-delete-fg">
          {state.error}
        </p>
      ) : null}

      {done?.cancelled ? (
        <p role="status" className="text-xs text-muted">
          {t("explain.cancelled")}
        </p>
      ) : null}

      {/* Structured result (parsed by the backend from the model output). */}
      {done?.ok && (done.summary || done.impact) ? (
        <div className="flex flex-col gap-2 rounded-lg border border-line bg-surface-2 p-3">
          {done.summary ? (
            <div>
              <p className="text-xs font-semibold text-fg">{t("explain.summary")}</p>
              <p className="mt-0.5 text-xs leading-relaxed whitespace-pre-wrap text-muted">
                {done.summary}
              </p>
            </div>
          ) : null}
          {done.impact ? (
            <div>
              <p className="text-xs font-semibold text-fg">{t("explain.impact")}</p>
              <p className="mt-0.5 text-xs leading-relaxed whitespace-pre-wrap text-muted">
                {done.impact}
              </p>
            </div>
          ) : null}
        </div>
      ) : null}

      {/* Raw stream (plain text only — React-escaped, never markdown-HTML). */}
      {state.streaming ? (
        <div
          ref={streamRef}
          role="log"
          aria-live="polite"
          aria-busy="true"
          className="max-h-48 overflow-y-auto rounded-lg border border-line bg-surface-2 p-3"
        >
          {state.text.length === 0 ? (
            <Skeleton className="h-3 w-2/3" />
          ) : (
            <p className="text-xs leading-relaxed whitespace-pre-wrap text-muted">
              {state.text}
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}
