//! Compare wizard — the home route. Two file pickers, Tauri drag-drop,
//! format auto-detect, Compare → run_diff → /diff/{jobId}, live job card.

import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useTranslation } from "react-i18next";
import { ArrowRight, CircleAlert, GitCompare } from "lucide-react";

import { FilePicker } from "@/components/compare/FilePicker";
import { JobProgressCard } from "@/components/compare/JobProgressCard";
import { RecentSessions } from "@/components/history/RecentSessions";
import { Button } from "@/components/ui/button";
import { TauriRequiredError, isTauri, pickDocument, runDiff } from "@/lib/tauri";
import { formatFromExtension } from "@/lib/format";
import { setLastJob } from "@/lib/jobStore";
import { loadOcrPref } from "@/lib/prefs";

export function HomePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [oldPath, setOldPath] = useState<string | null>(null);
  const [newPath, setNewPath] = useState<string | null>(null);
  const [dragActive, setDragActive] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const supported = (p: string | null): boolean => p !== null && formatFromExtension(p) !== null;
  const ready =
    supported(oldPath) && supported(newPath) && oldPath !== null && newPath !== null;
  const sameFile = oldPath !== null && oldPath === newPath;

  const browse = useCallback(
    async (side: "old" | "new") => {
      setError(null);
      try {
        const picked = await pickDocument();
        if (picked !== null) {
          if (side === "old") setOldPath(picked);
          else setNewPath(picked);
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  // Tauri drag-drop: fill empty slots in order; two dropped files fill both.
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDragActive(true);
        } else if (event.payload.type === "leave") {
          setDragActive(false);
        } else if (event.payload.type === "drop") {
          setDragActive(false);
          const paths = event.payload.paths;
          if (paths.length >= 2) {
            setOldPath((prevOld) => prevOld ?? paths[0] ?? null);
            setNewPath((prevNew) => prevNew ?? paths[1] ?? paths[0] ?? null);
          } else if (paths.length === 1) {
            const p = paths[0];
            if (p) {
              setOldPath((prevOld) => prevOld ?? p);
              setNewPath((prevNew) => (prevNew === null ? p : prevNew));
            }
          }
        }
      })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const compare = useCallback(async () => {
    if (!ready || sameFile) return;
    setStarting(true);
    setError(null);
    try {
      // Honor the settings OCR preference (localStorage) on every run.
      const snapshot = await runDiff(oldPath, newPath, loadOcrPref());
      setLastJob(snapshot.job_id);
      await navigate({ to: "/diff/$jobId", params: { jobId: snapshot.job_id } });
    } catch (e) {
      setError(e instanceof TauriRequiredError ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }, [ready, sameFile, oldPath, newPath, navigate]);

  const dropHint = dragActive ? t("wizard.dropActive") : t("wizard.dropAreaHint");

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-6 py-8">
        <header className="text-center">
          <h1 className="flex items-center justify-center gap-2 text-lg font-semibold tracking-tight text-fg">
            <GitCompare aria-hidden className="h-5 w-5 text-accent" />
            {t("wizard.title")}
          </h1>
          <p className="mt-1 text-sm text-muted">{t("wizard.subtitle")}</p>
        </header>

        <div
          className={
            dragActive
              ? "rounded-2xl border-2 border-dashed border-accent bg-accent-soft/40 p-2 transition-colors"
              : "rounded-2xl border-2 border-dashed border-transparent p-2 transition-colors"
          }
        >
          <div className="flex items-stretch gap-3">
            <FilePicker
              side="old"
              label={t("wizard.oldLabel")}
              path={oldPath}
              busy={starting}
              invalid={oldPath !== null && !supported(oldPath)}
              onBrowse={() => void browse("old")}
              onClear={() => setOldPath(null)}
            />
            <div className="flex items-center" aria-hidden>
              <ArrowRight className="h-4 w-4 text-subtle" />
            </div>
            <FilePicker
              side="new"
              label={t("wizard.newLabel")}
              path={newPath}
              busy={starting}
              invalid={newPath !== null && !supported(newPath)}
              onBrowse={() => void browse("new")}
              onClear={() => setNewPath(null)}
            />
          </div>
          <p className="mt-1 px-1 text-center text-xs text-subtle">{dropHint}</p>
        </div>

        <div className="flex flex-col items-center gap-2">
          <Button
            variant="primary"
            size="lg"
            busy={starting}
            disabled={!ready || sameFile}
            onClick={() => void compare()}
            aria-disabled={!ready || sameFile}
          >
            {starting ? t("wizard.comparing") : t("wizard.compare")}
          </Button>
          {!ready && (oldPath !== null || newPath !== null) && !sameFile ? (
            <p className="text-xs text-subtle">{t("wizard.missing")}</p>
          ) : null}
          {sameFile ? (
            <p className="flex items-center gap-1.5 text-xs text-severity-medium-text">
              <CircleAlert aria-hidden className="h-3.5 w-3.5" />
              {t("wizard.sameFile")}
            </p>
          ) : null}
          {error ? (
            <p role="alert" className="max-w-md text-center text-xs text-delete-fg">
              {error}
            </p>
          ) : null}
        </div>

        <JobProgressCard />

        {/* Phase-08: persisted comparison history. */}
        <RecentSessions />
      </div>
    </div>
  );
}
