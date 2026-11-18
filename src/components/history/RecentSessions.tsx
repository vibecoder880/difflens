//! Recent comparisons — persisted session history on the Home screen
//! (phase-08). Lists `list_sessions` rows newest-first: format badge, file
//! basenames, relative time, stat chips; click opens the persisted result
//! in the diff view (`/diff/session-<id>`); per-row delete with confirm.

import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ArrowRight, History, Trash2 } from "lucide-react";

import { ErrorCard } from "@/components/ErrorCard";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { FormatBadge, StatChip } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { fileBaseName, formatRelativeTime } from "@/lib/format";
import { isTauri, listSessions, deleteSession } from "@/lib/tauri";
import type { SessionSummary } from "@/lib/types";

/** Route namespace for hydrated sessions (mirrors the Rust get_diff fallback). */
export function sessionRouteId(id: string): string {
  return `session-${id}`;
}

export function RecentSessions({ limit = 8 }: { limit?: number }) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [pendingDelete, setPendingDelete] = useState<SessionSummary | null>(null);
  const [deleting, setDeleting] = useState(false);

  const query = useQuery({
    queryKey: ["sessions", limit],
    queryFn: () => listSessions(limit, 0),
    enabled: isTauri(),
    // Relative times would go stale without refetching; refresh gently.
    refetchInterval: 60_000,
  });

  const openSession = (session: SessionSummary) => {
    void navigate({
      to: "/diff/$jobId",
      params: { jobId: sessionRouteId(session.id) },
    });
  };

  const confirmDelete = async () => {
    if (pendingDelete === null) return;
    setDeleting(true);
    try {
      await deleteSession(pendingDelete.id);
      await queryClient.invalidateQueries({ queryKey: ["sessions"] });
    } finally {
      setDeleting(false);
      setPendingDelete(null);
    }
  };

  // Plain-browser preview: the history surface needs the Tauri runtime.
  if (!isTauri()) return null;

  const sessions = query.data;

  return (
    <section aria-label={t("history.title")} className="flex flex-col gap-2">
      <h2 className="flex items-center gap-2 text-sm font-semibold text-fg">
        <History aria-hidden className="h-4 w-4 text-subtle" />
        {t("history.title")}
        <span className="text-xs font-normal text-subtle">{t("history.subtitle")}</span>
      </h2>

      {query.isPending ? (
        <div className="flex flex-col gap-2" role="status">
          <Skeleton className="h-14 w-full" />
          <Skeleton className="h-14 w-full" />
          <span className="sr-only">{t("common.loading")}</span>
        </div>
      ) : query.isError ? (
        <ErrorCard
          detail={query.error instanceof Error ? query.error.message : String(query.error)}
          onRetry={() => void queryClient.invalidateQueries({ queryKey: ["sessions"] })}
        />
      ) : sessions !== undefined && sessions.length === 0 ? (
        <EmptyState
          icon={History}
          title={t("history.title")}
          body={t("history.empty")}
          className="py-6"
        />
      ) : sessions !== undefined ? (
        <ul className="flex flex-col gap-1.5">
          {sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              locale={i18n.language}
              onOpen={() => openSession(session)}
              onDelete={() => setPendingDelete(session)}
            />
          ))}
        </ul>
      ) : null}

      <Dialog
        open={pendingDelete !== null}
        onClose={() => (deleting ? undefined : setPendingDelete(null))}
        title={t("history.deleteTitle")}
      >
        <p className="text-sm text-muted">{t("history.deleteBody")}</p>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setPendingDelete(null)} disabled={deleting}>
            {t("common.cancel")}
          </Button>
          <Button variant="danger" busy={deleting} onClick={() => void confirmDelete()}>
            <Trash2 aria-hidden className="h-3.5 w-3.5" />
            {t("history.confirmDelete")}
          </Button>
        </div>
      </Dialog>
    </section>
  );
}

function SessionRow({
  session,
  locale,
  onOpen,
  onDelete,
}: {
  session: SessionSummary;
  locale: string;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  return (
    <li className="group flex items-center gap-2 rounded-xl border border-line bg-surface px-3 py-2 transition-colors hover:bg-surface-2">
      <button
        type="button"
        onClick={onOpen}
        title={t("history.open")}
        className="flex min-w-0 flex-1 items-center gap-2 text-left"
      >
        <FormatBadge format={session.format} />
        <span className="flex min-w-0 items-center gap-1 text-sm text-fg">
          <span className="truncate" title={session.old_path}>
            {fileBaseName(session.old_path)}
          </span>
          <ArrowRight aria-hidden className="h-3.5 w-3.5 shrink-0 text-subtle" />
          <span className="truncate" title={session.new_path}>
            {fileBaseName(session.new_path)}
          </span>
        </span>
        <span className="tabular hidden shrink-0 items-center gap-1 text-xs sm:flex">
          <StatChip
            glyph="+"
            count={session.stats.added}
            label={t("diff.stats.added")}
            tone="insert"
          />
          <StatChip
            glyph="−"
            count={session.stats.removed}
            label={t("diff.stats.removed")}
            tone="delete"
          />
          <StatChip
            glyph="~"
            count={session.stats.modified}
            label={t("diff.stats.modified")}
            tone="modified"
          />
        </span>
        <time dateTime={new Date(session.created_at).toISOString()} className="ml-auto shrink-0 text-xs text-subtle">
          {formatRelativeTime(session.created_at, locale)}
        </time>
      </button>
      <Button
        variant="ghost"
        size="icon"
        aria-label={`${t("history.delete")}: ${fileBaseName(session.old_path)} → ${fileBaseName(session.new_path)}`}
        onClick={onDelete}
      >
        <Trash2 aria-hidden className="h-3.5 w-3.5" />
      </Button>
    </li>
  );
}
