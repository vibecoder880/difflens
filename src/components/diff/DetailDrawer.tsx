//! Detail drawer for the selected change: full text, heuristic flags with
//! evidence, audit trail ("why this severity") and the AI explain panel.

import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ScrollText, X } from "lucide-react";

import { SeverityBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ExplainPanel } from "@/components/llm/ExplainPanel";
import { SpanText } from "@/components/diff/InlineSpans";
import { cn } from "@/lib/cn";
import { formatDelta } from "@/lib/format";
import { sideOnlyText } from "@/lib/diffRows";
import type { AuditEntry, Change, HeuristicFlag } from "@/lib/types";

function FlagRow({ flag }: { flag: HeuristicFlag }) {
  const { t } = useTranslation();
  return (
    <li className="flex flex-col gap-1 rounded-lg border border-line bg-surface-2 p-2.5">
      <div className="flex flex-wrap items-center justify-between gap-1.5">
        <span className="text-xs font-semibold text-fg">
          {t(flag.label_key, { defaultValue: flag.rule_id })}
        </span>
        {flag.delta !== null ? (
          <span className="tabular rounded bg-surface px-1.5 py-0.5 font-mono text-[11px] text-accent">
            {formatDelta(flag.delta)}
          </span>
        ) : null}
      </div>
      {flag.old_value !== null || flag.new_value !== null ? (
        <div className="flex flex-wrap items-center gap-1.5 text-xs">
          <span className="text-subtle">{t("drawer.flag.old")}:</span>
          <code className="max-w-full rounded bg-surface px-1 py-0.5 font-mono text-[11px] break-all text-delete-fg">
            {flag.old_value ?? "—"}
          </code>
          <span aria-hidden className="text-subtle">
            →
          </span>
          <span className="text-subtle">{t("drawer.flag.new")}:</span>
          <code className="max-w-full rounded bg-surface px-1 py-0.5 font-mono text-[11px] break-all text-insert-fg">
            {flag.new_value ?? "—"}
          </code>
        </div>
      ) : null}
      <span className="font-mono text-[10px] text-subtle">{flag.rule_id}</span>
      {flag.detail ? (
        <span className="text-[11px] text-muted">
          {t("drawer.flag.detail")}: {flag.detail}
        </span>
      ) : null}
    </li>
  );
}

/** Full text of one drawer side: spans, carried side-only text, or a fallback. */
function DrawerSideText({ change, side }: { change: Change; side: "old" | "new" }) {
  const { t } = useTranslation();
  if (change.spans.length > 0) {
    return <SpanText spans={change.spans} side={side} />;
  }
  const text = sideOnlyText(change, side);
  if (text !== null) {
    return <span className="whitespace-pre-wrap text-fg">{text}</span>;
  }
  const ids = side === "old" ? change.old_block_ids : change.new_block_ids;
  return (
    <span className="text-muted">
      {ids.length > 0 ? ids.join(", ") : t("drawer.noText")}
    </span>
  );
}

export function DetailDrawer({
  change,
  audit,
  diffJobId,
  onClose,
}: {
  change: Change;
  audit: AuditEntry[];
  diffJobId: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const closeRef = useRef<HTMLButtonElement>(null);
  const auditEntry = audit.find((a) => a.change_id === change.id);

  useEffect(() => {
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="absolute inset-0 z-30" role="dialog" aria-modal="true" aria-label={t("drawer.title")}>
      <button
        type="button"
        aria-label={t("common.close")}
        tabIndex={-1}
        onClick={onClose}
        className="absolute inset-0 bg-black/40"
      />
      <div className="absolute inset-y-0 right-0 flex w-[min(460px,92vw)] flex-col border-l border-line bg-surface shadow-2xl">
        {/* Header */}
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-line px-4 py-2.5">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold text-fg">{t("drawer.title")}</h2>
            <span className="tabular font-mono text-xs text-subtle">{change.id}</span>
            {change.severity !== null ? <SeverityBadge severity={change.severity} /> : null}
          </div>
          <Button
            ref={closeRef}
            variant="ghost"
            size="icon"
            aria-label={t("common.close")}
            onClick={onClose}
          >
            <X aria-hidden className="h-4 w-4" />
          </Button>
        </div>

        {/* Scrollable body */}
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-4 py-3">
          {/* Full text */}
          <section>
            <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-subtle uppercase">
              {t("drawer.oldText")}
            </h3>
            <div className="rounded-lg border border-line bg-delete-row p-2.5 text-xs leading-relaxed break-words">
              <DrawerSideText change={change} side="old" />
            </div>
            <h3 className="mt-3 mb-1.5 text-xs font-semibold tracking-wide text-subtle uppercase">
              {t("drawer.newText")}
            </h3>
            <div className="rounded-lg border border-line bg-insert-row p-2.5 text-xs leading-relaxed break-words">
              <DrawerSideText change={change} side="new" />
            </div>
          </section>

          {/* Flags */}
          <section>
            <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-subtle uppercase">
              {t("drawer.flags")}
            </h3>
            {change.flags.length === 0 ? (
              <p className="text-xs text-muted">{t("drawer.flagsEmpty")}</p>
            ) : (
              <ul className="space-y-1.5">
                {change.flags.map((flag, i) => (
                  <FlagRow key={`${flag.rule_id}-${i}`} flag={flag} />
                ))}
              </ul>
            )}
          </section>

          {/* Audit trail */}
          {auditEntry ? (
            <section className="rounded-lg border border-line p-3">
              <h3 className="flex items-center gap-1.5 text-xs font-semibold tracking-wide text-subtle uppercase">
                <ScrollText aria-hidden className="h-3.5 w-3.5" />
                {t("drawer.audit.title")}
              </h3>
              <div className="mt-2 flex items-center gap-2">
                <SeverityBadge severity={auditEntry.severity} />
                <span className="text-xs text-muted">{t("drawer.audit.why")}</span>
              </div>
              <p className="mt-1.5 text-[11px] text-subtle">{t("drawer.audit.ordered")}</p>
              <ol className="mt-2 space-y-1">
                {auditEntry.rules.map((rule, i) => (
                  <li
                    key={`${rule.rule_id}-${i}`}
                    className="flex items-center gap-2 text-xs text-muted"
                  >
                    <span className="tabular w-4 text-subtle">{i + 1}.</span>
                    <code className="font-mono text-[11px] text-fg">{rule.rule_id}</code>
                    <span>{t(rule.label_key, { defaultValue: "" })}</span>
                    {rule.delta !== null ? (
                      <span className="tabular font-mono text-[11px] text-accent">
                        {formatDelta(rule.delta)}
                      </span>
                    ) : null}
                  </li>
                ))}
              </ol>
            </section>
          ) : null}

          {/* AI explain */}
          <section className={cn("border-t border-line pt-3")}>
            <ExplainPanel diffJobId={diffJobId} changeId={change.id} />
          </section>
        </div>
      </div>
    </div>
  );
}
