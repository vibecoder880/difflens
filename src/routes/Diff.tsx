//! Diff view route: /diff/:jobId — header + virtualized side-by-side pane +
//! filterable change list + detail drawer with AI explain.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { FileQuestion, Loader2, SearchX } from "lucide-react";

import { ChangeList } from "@/components/diff/ChangeList";
import { DetailDrawer } from "@/components/diff/DetailDrawer";
import { DiffHeader } from "@/components/diff/DiffHeader";
import {
  SideBySideView,
  type SideBySideHandle,
} from "@/components/diff/SideBySideView";
import { ErrorCard } from "@/components/ErrorCard";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@tanstack/react-router";
import { EMPTY_FILTERS, type DiffFilters } from "@/lib/filters";
import { cancelDiff, getDiff, onDiffProgress } from "@/lib/tauri";

export function DiffPage() {
  const { jobId } = useParams({ from: "/diff/$jobId" });
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filters, setFilters] = useState<DiffFilters>({ ...EMPTY_FILTERS });
  const sideRef = useRef<SideBySideHandle>(null);

  const query = useQuery({
    queryKey: ["diff", jobId],
    queryFn: () => getDiff(jobId),
    refetchInterval: (q) =>
      q.state.data !== null && q.state.data?.state === "running" ? 1000 : false,
    retry: 1,
  });

  // Live progress: patch the cache from diff://progress between poll ticks.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void onDiffProgress((payload) => {
      if (payload.job_id !== jobId) return;
      queryClient.setQueryData(["diff", jobId], (old) => {
        if (old === null || old === undefined) return old;
        return { ...old, stage: payload.stage, percent: payload.percent };
      });
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
  }, [jobId, queryClient]);

  const snapshot = query.data ?? null;
  const result = snapshot?.result ?? null;

  const onSelectFromList = useCallback(
    (changeId: string) => {
      setSelectedId(changeId);
      sideRef.current?.revealChange(changeId);
    },
    [],
  );

  const onSelectFromView = useCallback((changeId: string) => {
    setSelectedId(changeId);
  }, []);

  const closeDrawer = useCallback(() => setSelectedId(null), []);

  const selectedChange = useMemo(
    () => result?.changes.find((c) => c.id === selectedId) ?? null,
    [result, selectedId],
  );

  // ------------------------------------------------------------------
  // Async surfaces: loading / error / not-found / running / failed
  // ------------------------------------------------------------------
  if (query.isPending) {
    return (
      <div className="flex h-full flex-col gap-3 p-4">
        <Skeleton className="h-8 w-96" />
        <div className="grid flex-1 grid-cols-2 gap-3">
          <Skeleton className="h-full" />
          <Skeleton className="h-full" />
        </div>
        <span className="sr-only" role="status">
          {t("diff.loading")}
        </span>
      </div>
    );
  }
  if (query.isError) {
    return (
      <div className="p-6">
        <ErrorCard
          detail={query.error instanceof Error ? query.error.message : String(query.error)}
          onRetry={() => void queryClient.invalidateQueries({ queryKey: ["diff", jobId] })}
        />
      </div>
    );
  }
  if (snapshot === null) {
    return (
      <EmptyState
        icon={SearchX}
        title={t("diff.notFound")}
        action={
          <Link
            to="/"
            className="inline-flex h-8 items-center rounded-lg bg-accent px-3 text-xs font-medium text-accent-fg hover:bg-accent-hover"
          >
            {t("nav.compare")}
          </Link>
        }
      />
    );
  }
  if (snapshot.state === "running") {
    return (
      <div className="flex h-full flex-col">
        <DiffHeader snapshot={snapshot} onCancel={() => void cancelDiff(jobId)} />
        <div className="flex flex-1 flex-col items-center justify-center gap-3">
          <Loader2 aria-hidden className="h-6 w-6 animate-spin text-accent" />
          <p className="text-sm text-muted">
            {t("diff.running")} · {snapshot.percent}%
          </p>
        </div>
      </div>
    );
  }
  if (snapshot.state !== "done" || result === null) {
    return (
      <div className="flex h-full flex-col">
        <DiffHeader snapshot={snapshot} />
        <div className="flex-1 p-6">
          <ErrorCard
            detail={snapshot.error ?? undefined}
            onRetry={
              snapshot.state === "failed"
                ? () => void queryClient.invalidateQueries({ queryKey: ["diff", jobId] })
                : undefined
            }
          />
          {snapshot.state === "cancelled" ? (
            <div className="mt-4 text-center">
              <Link
                to="/"
                className="text-xs font-medium text-accent hover:underline"
              >
                {t("wizard.recent.open")}
              </Link>
            </div>
          ) : null}
        </div>
      </div>
    );
  }

  if (result.changes.length === 0) {
    return (
      <div className="flex h-full flex-col">
        <DiffHeader snapshot={snapshot} />
        <EmptyState
          icon={FileQuestion}
          title={t("diff.empty")}
          action={
            <Link
              to="/"
              className="inline-flex h-8 items-center rounded-lg bg-accent px-3 text-xs font-medium text-accent-fg hover:bg-accent-hover"
            >
              {t("wizard.compare")}
            </Link>
          }
        />
      </div>
    );
  }

  return (
    <div className="relative flex h-full flex-col">
      <DiffHeader snapshot={snapshot} onCancel={() => void cancelDiff(jobId)} />
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          <SideBySideView
            ref={sideRef}
            changes={result.changes}
            selectedId={selectedId}
            onSelect={onSelectFromView}
          />
        </div>
        <ChangeList
          className="w-80 shrink-0"
          changes={result.changes}
          selectedId={selectedId}
          filters={filters}
          onFiltersChange={setFilters}
          onSelect={onSelectFromList}
        />
      </div>
      {selectedChange ? (
        <DetailDrawer
          change={selectedChange}
          audit={result.audit}
          diffJobId={jobId}
          onClose={closeDrawer}
        />
      ) : null}
      {/* Screen-reader hint for the select-to-inspect flow. */}
      <p className="sr-only">{t("diff.selectHint")}</p>
    </div>
  );
}
